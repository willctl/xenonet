# Contracts between the workstreams

Fixed in Phase 0 (2026-10-05) so the workstreams of the next phase don't
wait on each other: schema ids, host-qualified ids, what an actor looks
like on the wire, the theme tokens, and `state.json` v2. `next-phase.md`
has the plan they belong to. A change to anything here needs the
orchestrator's OK and a note to every lane it touches.

## 1. Schemas

### Ids

- JSON Schema draft 2020-12. Each schema's `$id` is
  `https://xenonet.dev/schemas/<path>/v<major>.json`. The ids don't
  need to resolve.
- Inside data, the short form `<path>/v<major>` names a schema, for
  example `"schema": "actor/snapshot/v1"`.
- Internal schemas, by path: `session`, `project`, `graph`,
  `live/snapshot`, `live/session`, `live/activity`, `identity`,
  `engine/message`, `engine/claim`, `engine/contact`, `engine/agent`,
  `doctor/result`, `doctor/run`, `theme`, `state` (v1 is today's file, v2
  the new one), `actor/snapshot`, `actor/event`, `actor/list`,
  `actor/registration`, `actor/credentials`, `actor/gateway-credentials`,
  `actor/report`, `actor/me`,
  `actor/events`, `actor/console`, `actor/requests`, `actor/tokens`,
  `drift/record`, `drift/log`, and for hosts `host/list` and
  `host/hello`.
- Vendor schemas, at `vendor/<vendor>/<product>/<source>`:

| Path | What |
|---|---|
| `vendor/anthropic/claude-code/transcript-line` | A line of a Claude Code transcript (`~/.claude/projects/**.jsonl`), subagents' included |
| `vendor/anthropic/claude-code/config` | The parts of `~/.claude.json` XenoNet reads |
| `vendor/anthropic/claude-desktop/session` | Claude desktop's `claude-code-sessions/**/local_*.json` |
| `vendor/anthropic/claude-desktop/plan-usage` | Claude desktop's `plan-usage-history.json` |
| `vendor/anthropic/agent-sdk/session-listing` | What the Agent SDK's `listSessions` returns |
| `vendor/anthropic/agent-sdk/session-message` | What `getSessionMessages` returns |
| `vendor/anthropic/oauth/usage` | The `/api/oauth/usage` response |
| `vendor/openai/codex/rollout-line` | A line of a Codex rollout (`~/.codex/sessions/**`) |
| `vendor/openai/codex/app-server` | The `codex app-server` methods XenoNet calls and the results it reads |
| `vendor/openai/codex/state-db` | The tables and columns XenoNet queries in `state_5.sqlite` |
| `vendor/openai/codex/auth-claims` | The ID token claims XenoNet reads from `auth.json` (never the tokens) |
| `vendor/github/gh/hosts` | gh's `hosts.yml`, hosts and logins only |

  Actors add their own in Wave 2, such as `vendor/nous/hermes-agent/state-db`.

### Versions

- The `$id` carries the major version. Adding an optional field, a new
  value of an enum or a new variant keeps the major, so other machines'
  kinds of node grow `graph/v1` rather than making a v2. Removing,
  renaming or retyping a field makes a new major, and data XenoNet has
  stored gets a migration.
- A vendor schema carries
  `"x-xenonet": {"vendorVersions": "<semver range>", "lenient": true}`.
  When a vendor changes a format, the schema gets a new major with its
  own range, and the adapter picks by the version it detects (or tries
  the newest first when the data carries no version).
- What XenoNet writes says what it is: `state.json` has `"version": 2`;
  `doctor.json`, drift records, script files and actor payloads have a
  `"schema"` field. Existing API responses keep their shape; tests check
  them against the schemas instead.

### Layout

- `core/schema/`: the definitions, the source of truth, one module per
  area (`core/schema/vendor/<vendor>/<product>.js` for vendors), and
  `core/schema/ids.js`, which builds `$id`s and parses and qualifies host
  ids (section 2).
- `schemas/`: the generated JSON Schema files, mirroring the `$id` paths,
  committed. `npm run schemas` regenerates them, and a unit test fails
  when they're out of date.
- `docs/design/schemas.md`: the layout and the rules, and
  `docs/design/adr/0001-schemas.md`: zod or hand-written, and why.

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
  file, and shapes are cached. The benchmark holds tailing and refresh
  within 10% of the baseline.

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
  lookedAt, records}`; at most 500 records, oldest
  dropped first). `npm run contracts` reports them, and the doctor warns
  about drift seen since you last looked. Only `xenonet drift` and the
  console mark drift as looked at, never `npm run contracts`, which agents
  run too; it keeps what it finds in the `drift.json` of the XenoNet it runs
  as (XenoNet Dev's, from a checkout). `xenonet drift --exit-code` exits 1
  when there's new drift, and leaves it unmarked, so a script goes on
  exiting 1 until you look with plain `xenonet drift` or the console.

## 2. Host-qualified ids

- Every XenoNet gets a host id the first time hosts are connected:
  `^[a-z][a-z0-9-]{1,31}$`, a name you choose, such as `studio` or `vm1`.
  It never defaults to the machine's own name, which often holds a
  person's name. A host names itself (`xenonet hosts self <id>`); a console
  uses that name unless it clashes, and may rename the host for itself. A
  random host key, made once, recognizes a host across renames and SSH
  aliases.
- An unqualified id means this XenoNet's own host. One machine works exactly
  as today: node ids, layout keys and pins don't change.
- A node from another host is `@<host>:<local id>`:
  `@vm1:claude:4f1c0000-0000-4000-8000-000000000001`,
  `@vm1:project:~/projects/atlas`. To parse one: it starts with `@`, and
  the host runs to the first `:`. A host turns its path keys into `~`
  before they leave it, so no home folder (a user name) crosses machines
  or lands in the console's layout keys. Local ids stay as they are.
- A project's `repoKey` is computed on its own host: its SSH aliases
  resolved, `ssh://`, ports and user names dropped, then `normalizeRemote`.
  A project merged across hosts by it is `repo:<repoKey>`, for example
  `repo:github.com/willctl/xenonet`, kept for a merged view.
- Identities have no host: account ids are already global, so one Claude
  account signed in on two machines is one identity. The exception is
  `openai:api-key`, which belongs to its host and is qualified. Plan
  limits in the live snapshot name the identity they belong to.
- Actors are `actor:<slug>` (`^[a-z0-9][a-z0-9-]{0,31}$`), or
  `@<host>:actor:<slug>` from another host. The person at the console is
  `actor:you`.
- Being a host needs no flag: `xenonet relay` and `GET /api/hosts/self`
  work on any XenoNet. Showing other hosts needs `XENONET_HOSTS=1` (a
  prototype). The relay sends titles, states and counts unless a host was
  added with `--detail`, a subagent's task among the detail; transcripts,
  stats and diffs cross only when one is opened. A console keeps what a
  host sends only as the schemas declare it (`core/hosts/declared.js`),
  so no field of a host's own reaches the page. A demo console shows demo
  hosts only. Adding and removing hosts is yours: XenoNet refuses it from
  under any agent. Remote hosts are read-only.

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
    { "id": "project:/users/will/projects/atlas-api", "relation": "works-on", "since": 1791231000000, "lastAt": 1791234500000 },
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
| `via` | How XenoNet knows of it: `observed` (reads its state), `attributed` (traces work here back to it), `registered` (it reports itself), `demo` |
| `presence` | `working`, `idle`, `away` (missed two heartbeats), `offline`, `unknown`. "Working for 90 seconds after an action" counts only a registered actor's own events. You go offline ten minutes after XenoNet's window is hidden |
| `status` | What it says or XenoNet infers it's doing, at most 140 characters, or null. A registered actor's `status` events may carry its own line here, as plain text |
| `needsYou` | `{kind, text, since}` when the actor waits on you (it reaches the alert strip), else null or absent. `kind` is an open set (`question`, `permission`, `review`); `text` is at most 140 characters, and "Waiting for you" stands in for a line that's refused |
| `capabilities` | Any of `spawn-sessions`, `edit-repos`, `run-commands`, `message-people`, `message-agents`, `schedule`, `browse`, `use-computer` |
| `targets` | Graph node ids it's acting on (never actor ids; an event may still target an actor), each with a `relation`: `works-on`, `spawned`, `edited`, `messaged`, `watches`, `reviews` |
| `recent` | Its last events, newest last, at most 20 |
| `identity` | An identity id (`claude:…`, `chatgpt:…`, `github:…`) if XenoNet knows which account it acts as |

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
  (docs/guide/refusals.md); `live/activity` gains the `refusal` kind. SSE
  `assistant`: `{at}`, only that a
  tunnel XenoNet runs changed; the window reads how with its key
  (`GET /api/assistant/tunnels`).
  SSE `actor-requests`: `{at, waiting}`, counts only, so who asked stays
  the window's to see.
- `GET /api/actors/you`: with the window key, your whole
  `actor/snapshot/v1` (with `?since=`, your `actor/events/v1`); without
  it, the public placeholder the list shows; with a wrong key, 403
  `yours`. `POST /api/actors/you` takes the window's reports and answers
  an `actor/me/v1` with `unread: 0`. Turning off "Show me on the map"
  deletes your record: `DELETE /api/actors/you`, with the window key,
  answers `{"deleted": "actor:you"}`.
- Window only: `GET /api/actors/requests` answers an `actor/requests/v1`
  (each program waiting for an Allow: what it asks, who asked, when it
  lapses). `GET /api/actors/tokens` answers an `actor/tokens/v1`:
  `{"schema": "actor/tokens/v1", "at": <ms>, "tokens": [{actor, state,
  expiresAt, usedAt}]}` for each actor that reports itself, oldest first,
  where `state` is `works`, `expired` (30 days unused) or `none` (the
  demo's made-up actor), `expiresAt` is when it stops working unless it's
  used (null unless it works), and `usedAt` is when it was last used,
  exactly the first time and to within the hour after, or null for a
  token never used, which a new token is again. Never a token, nor its
  hash. `GET /api/actors/observers` lists the observers and whether each
  is on, for any local program.
- Driver (`src/driver.js`): `actors()`, `actorRequests()`, `actorTokens()`
  (how each token stands, as the window reads it) and `feed()` (the
  activity feed's lines, sessions' and actors'), and `select("actor:…")`;
  `find` matches actors as kind `actor`.
- Console tools (`mcp/console.js`): an `actors` tool, which brings the
  `requests` and the `tokens` too, and a `feed` tool; `simulate` gains
  `actor-spawn`, `actor-edit`, `actor-message`, `actor-online`,
  `actor-offline`, and in the demo and test instances only
  `actor-request` and `actor-needs-you` (an agent waiting on you with a
  made-up question, its card in the attention strip, until `clear`), and
  `actor-tunnel`, `actor-tunnel-down` and `actor-read` (a made-up
  connection through a tunnel, its gate open or shut, and a read
  through it). The `actors` tool brings `tunnels` too: each gate drawn,
  how open and where.
- The demo world has `actor:you` and three made-up agents with scripted
  activity: a local agent that works on projects and spawns sessions, a
  cloud agent that works through an app on this machine, and a custom
  actor that registered itself. Their names are made up too.

### Registering

`docs/design/actors/interface.md` has the full API, accepted at merge
point A, and Wave 2 builds it. The essentials:

- Registering answers `202` with an id and a token that works only once
  you Allow the actor in XenoNet's window; every new actor waits for that,
  even one you register from your own terminal. Owner-only routes take a
  key the server gives only its own window.
- `actor/registration/v1` gains an optional `slug`. The engine's schemas
  allow `actor` as an `app` (`engine/agent`, the brief in `engine/message`,
  `engine/contact`).
- The `xenonet` MCP server gets `actor_heartbeat`, `actor_events` and
  `actor_targets`, but nothing to register or unregister: those stay
  yours.
- A token unused for 30 days is revoked as `expired`. Over the bad-token
  limit, only that process's further bad tokens get 429 for a minute.
- An actor XenoNet knows only by observing it can't be removed, whoever
  asks: a 409 with the code `observed`, whose body also names its
  `product`. Its observer's switch takes it off the map. Nor can it be
  messaged, since it reads no inbox: `send` refuses it. Installed products
  stay listed however long they haven't run, as offline, except Grok Bot,
  whose Bots work in the cloud with its app closed: it's listed as
  unknown for as long as it's installed, and never offline.
- An observed actor's edited paths (Hermes's, read with `json_extract`)
  count in agents' edit warnings and `who_has` for 30 minutes, like a
  registered actor's, marked as observed.

The proposal from Phase 0, which the interface doc refines:

- `POST /api/actors/register` with an `actor/registration/v1` body
  (`{name, product?, capabilities?, heartbeatMs?, pid?}`) returns an id and
  a token, shown once and stored hashed.
- Then `POST /api/actors/heartbeat {presence?, status?}`,
  `POST /api/actors/events {events}`, `POST /api/actors/targets {targets}`
  and `DELETE /api/actors/me`, each with `Authorization: Bearer <token>`.
  On macOS the process check in `core/engine/verify.js` applies too.
- The same calls as `actor_*` tools on the `xenonet` MCP server, and as
  `xenonet actor …` on the command line.
- `actor:<slug>` is a valid agent id for the engine's messages and claims.
  Actors can't approve anything, the same as agents.

## 4. Theme tokens

New fields, all optional. A missing field takes XenoNet's value, so every
theme file people already have stays valid, and `theme/v1` only grows.

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
| `motion.pace` | A multiplier for ambient motion (1 is today's) |
| `motion.ease` | `smooth`, `stepped` (everything moves in ticks) |
| `motion.breathe` | How much nodes swell and fade at rest: 0.1 is today's bob (4.5 × breathe), 0 holds still |
| `stars.shape` | `point`, `flake`, `mote` |
| `stars.drift` | Units per second; `[0.6, -4, 0.3]` is falling snow at the map's scale |
| `grid.style` | `polar`, `hex`, `cells`, `none` |
| `sweep.shape` | `beam`, `aurora`, `flare`, `hand`, `ring`, `none` |
| `sweep.opacity` | A strength relative to XenoNet's beam (0.03), so a theme that only changes the shape still shows it |
| `sonar.style` | `ring`, `ripple`, `ping`, `tick` (drawn as the ring for now) |
| `sonar.every` | Seconds between the core's waves, never under one apart; 0 or less sends none |
| `lines.style` | `solid`, `dashed`, `arc`, `filament`, `seam`, `hairline` |
| `fonts.body` | The font for body text in the panels; `fonts.display` when left out. Every `fonts` value is a list of font names, quoted or not: one holding anything but letters, digits, spaces, quotes, commas and `. _ + & -` keeps the value underneath, since it could end the style it goes in |
| `identities` | Five ring colors for named identities; derived as today when left out |
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
- The themes lane fills these in for every theme (the themes from before
  keep their exact look), and the actors UI lane draws the forms and may
  tune the values.

## 5. `state.json` v2, and changing the root

- v1 is today's file, with no version. v2 adds `"version": 2`,
  `"workspace": {"root": null}` (a path, or null for
  `XENONET_PROJECTS_DIR` or `~/Projects`), and keeps Codex's picks per root:
  `"projects": {"rules": {...}, "codexByRoot": {"<root path key>": <picks>}}`.
  Migrating moves v1's `projects.codex` under the key of the root it was
  made for.
- On a root change:

| Part | What happens |
|---|---|
| Layout | Kept as it is. Node ids are absolute (`project:<path key>`), so going back to a root brings its positions back |
| Your rules | Kept, keyed by absolute path; they apply under whichever root holds them. The rules list can undo any rule, inside the root or not |
| Codex's picks | One set per root, each with its fingerprint; never applied to another root's folders |
| Rescan | The projects scan, the git status loop (dropping repos outside the root), identities' repo scan and the graph, in one pass. Transcripts, sessions and the engine don't change; sessions outside the new root become zones, as today |

- Precedence: `XENONET_PROJECTS_DIR` wins when it's set (the picker says
  so), then `state.json`, then `~/Projects`.
- Safety: the first load that migrates writes `state.v1.json` beside the
  file. A `state.json` that isn't valid JSON is moved aside
  (`state.broken-<time>.json`) and reported by the doctor, never replaced
  by `{}` and overwritten. A file with a newer version than the code knows
  is read as it is and never written back with an older shape.
- The core lane writes the version, migrations and backups; the root lane
  writes the workspace accessors against this shape. Both touch
  `core/store.js`, and the orchestrator joins them at the merge.

## 6. Shared files

| File | Lanes | Rule |
|---|---|---|
| `core/store.js`, `core/state.js` | core, root | core: versions, migrations, sources on blocks. root: workspace and rescan |
| `server/index.js` | core, root, themes | New routes go in `server/routes/<area>.js`; `index.js` only mounts them |
| `src/main.js`, `src/palette.js` | themes, root | New UI goes in new modules (`src/themes-ui.js`, `src/root.js`); `main.js` only registers them |
| `src/driver.js`, `mcp/console.js`, `core/script.js` | themes, root | Add methods, tools and steps; don't rewrite existing ones |
| `test/e2e/` | themes, root | New files only (`themes.test.js`, `root.test.js`) |
| `README.md` | all | themes owns Themes, root owns Finding projects; the rest waits for Wave 3 |
| `package.json` | themes (fonts), core (scripts) | Add entries only |

## 7. Working rules for the lanes

- Work only in your worktree (`.claude/worktrees/<lane>`), on its branch.
  Use absolute paths, and `cd` into the worktree for every shell command.
  Never touch main or another worktree, and push nothing.
- Commit small Conventional Commits as Will <will@willctl.com> (check
  `git config user.email` once), each passing `npm test`, in the style of
  `git log`: a plain subject, a body that says why, and the
  `Co-Authored-By` trailer.
- Don't run `npm run test:e2e`, and don't launch the `dev` or `demo`
  instance. Only a wave's UI lane launches the `test` instance, with its
  worktree's own `node cli/xenonet.js console --instance test …` (the
  `xenonet-console` MCP tools run the main checkout's code). Write new e2e
  tests; the orchestrator runs them at the merge point.
- Run `node scripts/bench.mjs graph refresh tail server` before and after a
  change to a hot path, and say what moved. Flag anything more than 10%
  slower.
- No new dependency, install, download or sign-in: return it as a question
  instead.
- Fixtures and examples are made up. Real data on this machine may be
  read for shapes only (keys and types, never values), and nothing from it
  is copied into the repo. Paths under the home folder show as `~`.
- Write tests first where a subsystem you change has none.

## 8. Changes at merge point A (2026-10-05)

Accepted with every recommendation at merge point A (see
`next-phase.md`). The sections above already say them; this is the list.

- Schemas (§1): `actor/credentials`, `actor/report`, `actor/me`,
  `actor/events`, `actor/console`, `drift/log`, `host/list` and
  `host/hello`. Drift merges across vendor versions, counts unknown keys
  only in closed objects, and splits notes (additions) from warnings
  (changes).
- Hosts (§2): hosts name themselves, with a random host key; remote path
  keys are tilded on their host; `repoKey` is computed on the host and
  `repo:` is kept for a merged view; `openai:api-key` is qualified; plan
  limits name their identity.
- Actors (§3): the `use-computer` capability; `needsYou` on a snapshot;
  targets are graph node ids only; a registered actor's `status` events
  carry its own line; registration as `interface.md` has it (Allow in
  XenoNet's window, the window key, an optional `slug`, the engine's schemas
  allow `actor`).
- Themes (§4): `fonts.body`, `identities` ring colors, and clearer
  meanings for `motion.breathe`, `stars.drift`, `sweep.opacity` and
  fallback values.

## 9. Changes at merge point B (2026-10-05)

Accepted with every recommendation at merge point B (see
`next-phase.md`). The sections above already say them; this is the list.

- Schemas (§1): `actor/requests`; an optional `variant` on drift records;
  a new enum value is additive, so hosts grow `graph/v1` (a `host` node
  and link kind, a `same-repo` relation, `hosts` and `self` on the graph,
  `host`, `stale` and `repoKey` on nodes), `identity/v1` (`hosts`,
  `signedInOn`), `live/session/v1` (`host`, `stale`, `needsYou.asOf`),
  `live/snapshot/v1` (`identity` and `host` on readings, `limits.others`),
  `live/activity/v1` (`host`) and `host/list/v1` (`start`). Only `xenonet
  drift` and the console mark drift as looked at.
- Hosts (§2): no flag to be a host; `XENONET_HOSTS=1` to show them;
  `--detail` opts in to more than titles, states and counts; demo consoles
  show demo hosts only; adding and removing hosts is yours.
- Actors (§3): the `actor-requests` event; `GET` and `POST
  /api/actors/you`; the window's requests and the observers' routes;
  `needsYou` kinds and text; presence after an action and with the window
  hidden; your record deleted when you turn yourself off; tokens expiring
  after 30 days; observed actors not removable, kept listed while
  installed, and counted in edit warnings.

## 10. Changes in Wave 3 (2026-10-06)

Made in Wave 3, by the owner's decisions at merge point B and the calls
agreed in its review (see `next-phase.md`). The sections above already
say them; this is the list.

- Schemas (§1): `actor/tokens`, and `actor/requests` defined from what
  its route answers; `engine/agent/v1` declares an `observed` edit;
  `live/session/v1` gains `interruptedAt` and `live/activity/v1`'s
  turn-end `interrupted`, for a Claude interruption and a Codex turn
  aborted as interrupted or with no reason. `xenonet drift --exit-code`
  leaves drift unmarked, and `npm run contracts` keeps what it finds in
  the `drift.json` of the XenoNet it runs as, without marking it. Drift
  keeps less of what could be data: keys that are tokens (by their
  prefixes) or hold a hex id, no key's name inside a declared record,
  and a vendor version only when it reads as one. The vendor schemas know
  the newer kinds of line Claude Code and Codex write.
- Hosts (§2): what a host sends is kept only as the schemas declare it,
  and a subagent's task crosses only with `--detail`.
- Actors (§3): the 409 code `observed`, with `product`; `DELETE
  /api/actors/you` answering `{"deleted": "actor:you"}`; the window's
  `GET /api/actors/tokens` and `actor/tokens/v1`; `actor-needs-you`; the
  driver's `feed()` and `actorTokens()`, the console's `feed` tool and
  the `actors` tool's `tokens`; observed-only actors can't be messaged;
  Grok Bot listed as unknown while installed, never offline, where other
  installed products are listed as offline. Registration's rules for
  names and products (look-alike letters, one script, no maker's name)
  are in `interface.md`, section 2, and `tokens` joins the reserved slugs.
- Themes (§4): `sonar.every` of 0 or less sends no sonar, and waves are
  never under a second apart; `fonts` values are lists of font names.

## 11. Remote actors (2026-10-07)

Remote actors read through connections the owner pairs (design:
`cloud-assistants.md`; guide: `docs/guide/cloud-assistants.md`).

- Schemas (§1): `assistant/result/v1` and `assistant/error/v1`, the
  envelopes of the six tools of `xenonet mcp assistant`, whose answers
  carry `"schema": "xenonet.assistant.result.v1"` and
  `"xenonet.assistant.error.v1"`; and `actor/connection/v1`, the owner's
  view of a connection. Unlike every other schema they're strict
  (`additionalProperties: false`): data that leaves the machine is an
  allowlist, and a field the contract doesn't name is refused, in
  arguments and answers alike. A new field is a new version.
- Ids: a device is `dev_<uuid>`, a project as remote readers see it
  `prj_<uuid>` (node ids hold paths), a grant `gr_<24 hex>`, a
  connection `cn_<24 hex>`, an attention item `att_<24 hex>` and a
  request `r_<uuid>`. Sessions keep their `claude:`/`codex:` ids when
  they're UUIDs.
- Actors (§3): `via` gains `remote`; `actor/snapshot/v1` gains an
  optional `remote` (connections, active, bridge, accountLink,
  permissions, and `transports`, the routes its active connections
  take), left out for an actor without one. `identity` stays null
  for a link that's only the owner's word. Products gain `remote` beside
  `registers`, and `dots-<suffix>` slugs are dots'.
- `engine.sqlite` version 6 adds a message's `envelope`
  (`message/envelope/v1`, core/engine/attest.js): how XenoNet knew who
  sent it, from its own checks; the inbox and `engine/message/v1` carry
  it, and the agent-message event its `verified` line and `strength`.
  `POST /api/actors/you/message {to, text}`, with the window key, writes
  to an actor as you.
- `engine.sqlite` version 5 adds `actor_connections`, `actor_grants`,
  `actor_account_links`, `actor_external_refs`, `assistant_places`,
  `assistant_meta` and `assistant_audit`, with foreign keys on.

## 12. Integration points (2026-10-07)

What XenoNet writes into other programs outlives any one version of its
Desktop, Service and Tools, so these stay as they are, and a change to
one is a new version of it beside the old, never an edit:

- The launcher, at `<state>/runtime/launcher/`: its own `node` and
  `cli/xenonet.js` (`core/components/launcher.cjs`). Hooks, MCP servers,
  the `xenonet` command on PATH, `ssh … xenonet relay`, actors' scripts and
  OpenAI's tunnel-client all run this path, so it never moves. Its first
  line says its version (`// xenonet-launcher <n> (protocol <p>)`); XenoNet
  replaces it only with a newer one, through a temporary file and a
  rename. `--launcher` prints `{launcher, protocol, tools, node}`.
- Its argument forms: `hook <app>` (protocol 1: the event on stdin is
  posted to `/api/hook/<app>` on `XENONET_PORT`, or `--port`, and the
  answer printed; always exit 0, nothing on stderr; it runs in the
  launcher itself, without the Tools component), `mcp`, `mcp console`,
  `mcp assistant --grant <id> [--port <n>]`, `relay`, `actor …`,
  `disconnect --yes --port <n>`. Without Tools, any form but `hook`
  exits 1 with one line on stderr.
- `XENONET_PORT` says which XenoNet; each instance has its own port
  (`core/instances.js`). Hook routes are `/api/hook/claude` and
  `/api/hook/codex`; a change to what they take is a new route beside
  them.
- Entries in Claude Code's and Codex's settings belong to the XenoNet whose
  port they name. Connecting or disconnecting one XenoNet changes its own
  and those whose files are gone, never another live XenoNet's. MCP
  servers are named by instance: `xenonet` and `xenonet-console` for the
  installed XenoNet, `xenonet-<instance>` (`xenonet-dev`) for the others.
  Unchanged entries are left byte for byte, which keeps Codex's trust in
  its hooks.
- Uninstalling with your data removed runs `disconnect` first, so no
  hook is left running a deleted launcher.
