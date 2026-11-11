# Rust core

XenoNet's core is Rust: one cargo workspace at the repository's root that
builds one binary, `xenonet`. That binary is the service, the command, the
hooks and the MCP servers. This doc says how the workspace is laid out,
which crate may depend on which and how that's checked, which processes
run, and what the service, the MCP servers and the hooks hold to.
[direction.md](direction.md) says where the core is going and
[goals.md](goals.md) what it's for; [development.md](../guide/development.md)
has the commands, the golden files and coverage.

## What the core is and isn't

The core is everything under `crates/` and `adapters/`. What stays JS, or
another language, is what runs in a window or drives one:

- `electron/`: Desktop, its window, tray, terminals' view and control
  channel, and the Node modules it runs itself (`electron/lib/`).
- `src/`: the console page, with `src/shared/`, the modules the page and
  Desktop both import.
- `test/`: the unit tests of the page, Desktop and the scripts, the
  end-to-end tests (`test/e2e/`) and the checks against real apps
  (`test/real/`).
- `scripts/`: the build and development tools.
- `tray/macos/`: XenoNet Tray, in Swift.
- `xenolith/`: Xenolith, with a Rust workspace of its own
  (`xenolith/rust/`, edition 2024), excluded from this one.

Where the Rust needs data the JS defines, a script writes it and a test
checks it is current: `node scripts/apps.mjs` writes
`crates/xenonet-adapter/src/apps.json` from `src/shared/apps.js`, and
`node crates/xenonet-console/themes.mjs` writes
`crates/xenonet-console/src/themes.json` from `src/shared/themes.js`. The
command keeps its own copy of the units the page prints
(`xenonet-text`'s `units`).

## The workspace

The root `Cargo.toml` takes every crate in `crates/*` and `adapters/*`,
with cargo's resolver 3, which picks dependency versions that build on the
declared `rust-version`. The crates are edition 2021 and declare Rust 1.88,
the oldest that `rmcp` builds on. `fuzz/` is a workspace of its own, since
libFuzzer needs a nightly Rust.

### Crates

Each crate has one job. "Depends on" lists the XenoNet crates it uses
(`xenonet-` left off), without its dev-dependencies.

**Model and formats.** No IO, and no vendor.

| Crate | Its job | Depends on |
| --- | --- | --- |
| `xenonet-text` | The formats XenoNet writes and reads, which its files, answers and the golden files pin character for character: JSON (numbers, key order, escapes), text, UTF-16 strings, ISO times, units, orderings, URL encoding, JSON values read field by field | nothing |
| `xenonet-model` | The data model: typed ids, every document XenoNet keeps, serves or carries between machines as a serde type, the operations' kinds and the action vocabulary, the seam between the engine and the network (`ops`: `Net`, `Sink`), the selector grammar, the words a start says to its asker, a call's tokens and their API prices. `schemas/` is written from these types ([schemas.md](schemas.md)) | text |
| `xenonet-schema` | Vendor data read leniently against `schemas/vendor/`, which keys are fields, the vendor readers, the drift log, and XenoNet's own documents read as their types | os, text |

**The machine.**

| Crate | Its job | Depends on |
| --- | --- | --- |
| `xenonet-os` | Files, paths, processes started with no window, locks that outlive a panic (`sync`), the error log, XenoNet's own folders, the home folder and platform, random ids, `fetch()` over HTTPS, a program's output lines, the programs that open things, the system panel's numbers, and `Failure`, why something didn't give its value | text |
| `xenonet-blocks` | The building blocks the subsystems are made of ([blocks.md](blocks.md)): lifecycle and health, scanners, tailers, normalizers, JSON stores and versioned SQLite | os, text |

**The registry and the adapters.**

| Crate | Its job | Depends on |
| --- | --- | --- |
| `xenonet-adapter` | How a platform plugs in: the `Platform` trait and the `Registry`, the registry of apps (`apps.json`) and actor products, and what an adapter brings: process rules, ways in for delivery, connect formats, doctor checks, readers, observers, openers, and where the desktop apps keep their folders | os, text, blocks, model, schema |
| `adapters/claude-code`, `codex`, `grok-build`, `opencode` | A platform each: its processes, its ways in for live delivery, its connect, its readers and its doctor checks | adapter, os, text, sources; codex also blocks, model, schema; grok-build also model, claude-code; opencode also schema |
| `adapters/cursor`, `gemini-cli`, `copilot` | The reader of each app's own records | adapter, os, text, sources; cursor also schema |
| `adapters/hermes`, `grok-bot`, `muse` | The observer of each actor product; Hermes also XenoNet's plugin for Hermes and the doctor's check on it | adapter, os, text; hermes also actors, schema |
| `adapters/all` (`xenonet-platforms`) | Every adapter, in the registry's order, and `install()`, which puts them in place; and `Registry`, the registry as the HTTP routes ask it (`xenonet-api`'s `Apps`) | adapter, api and every adapter |

**The core's domains.**

| Crate | Its job | Depends on |
| --- | --- | --- |
| `xenonet-sources` | What XenoNet reads: transcript lines and their events, the records readers make, live telemetry, Codex's archived rollouts, the shared context, plan usage, and the refusals XenoNet keeps (with the Refusals core) | adapter, model, os, text, schema, api |
| `xenonet-state` | The map's data and what XenoNet remembers: `state.json` and its versions, the graph, search, stats and digests, projects and identities, the decision model's checks; and the map's runtime | adapter, api, blocks, components, demo, model, os, schema, text, sources |
| `xenonet-engine` | Which process sent a request (`procs`, `verify`), messages, claims and views, the index of the agents it knows, live delivery, the network's side of them, and the engine's SQLite store | adapter, model, os, text, blocks |
| `xenonet-actors` | The programs you allow in XenoNet: their registry, registration, reports, wakes, remote connections, tokens and token files; and the secret stores | adapter, model, os, text |
| `xenonet-net` | One network across machines ([network.md](network.md)): a host's keys, signed and sealed frames, the roster chain, tickets, the gateway's routes and queues, a host's link to its gateway, services through the network | model, os, text |
| `xenonet-assistant` | What remote actors may read ([cloud-assistants.md](cloud-assistants.md)): grant credentials, page tokens, the projection a grant sees, the six reads, the tunnels; and their runtime | os, text, components, api, engine, actors, adapter, model, schema |
| `xenonet-components` | XenoNet's install: the command on PATH, the service's settings, XenoNet Dev's instances, the version, the Service and Tools packages (checked, unpacked, switched, rolled back), starting the service at login, and the launcher | os, text |
| `xenonet-integrations` | Routing suggestions, OpenShell sandboxes and their reviews, Pi sessions, MCP servers' catalogs, the decision model and OpenRouter's spend; and their cores | os, adapter, text, api, actors, sources, state |
| `xenonet-doctor` | Each check's verdict and wording, the summary, new problems, the diagnostics bundle; and the Doctor and Diagnostics cores | components, adapter, os, text, schema, api, engine, model, net, sources |

**The runtime.** What runs the domains on this machine: tokio tasks,
timers, child processes, files and the waits between them. Most of it is a
`runtime` module in its domain's crate; the engine's is a crate of its own.

| Crate or module | Its job | Depends on |
| --- | --- | --- |
| `xenonet-runtime` | The engine running here: its store's sides, the map's readers, the process table and its checks, the engine and delivery's drivers, the network across machines, starts, asks and runs, the actors' sides (registration, wakes, reports, tokens, spend, observers), the local door's decisions through `xenonet-authz` (`door`), and other machines of the network on the map (`remote`: the relay, each host's link and the merge that qualifies what a host sends, with the hub that runs them) | adapter, text, api, os, actors, blocks, engine, schema, model, authz, id, log, net |
| `xenonet-state`'s `runtime` | The map kept current: the refresh from the vendor files, the live tailer, identities, projects, search, stats and the demo's world | (its crate's) |
| `runtime` in `xenonet-assistant`, `-doctor`, `-integrations`, and `xenonet-sources`' `refusals::runtime` | Each domain's cores, as the service runs them | (their crates') |
| `xenonet-demo` | The made-up world the demo runs on: projects, transcripts, rollouts and a shared context in a temporary folder, kept busy | os, text |

**The api.**

| Crate | Its job | Depends on |
| --- | --- | --- |
| `xenonet-api` | The local protocol between the service and the drivers of its cores, and the only XenoNet crate the HTTP surface imports besides the base crates: the core traits the routes call (`cores/`, JSON in and out), the local door (`LocalDoor`) and the app registry (`Apps`) as the routes ask them, what every driver is given (`wire.rs`: settings, the `Emitter` and `EventSink` that carry events to the service, the drift log), a message's envelope (`attest`), how a request carries a token (`bearer`), rates (`limits`), the Needs you queue, the event stream a relay follows, who sent a request, and the HTTP client of a running XenoNet that the command, the hook and the MCP servers use | model, os, text, net, schema |

**The surfaces.**

| Crate | Its job | Depends on |
| --- | --- | --- |
| `xenonet-http` | The local HTTP API on 127.0.0.1: the routes, the rules table each is checked by before its handler runs (`routes/rules.rs`), the requests and answers, the event stream, the cores' events on their way to the pages, the page's files, and the traits of the service's parts it reaches (`parts.rs`: the actor gateway, the component service, the assistant's endpoint, A2A, the network) | api, model, os, text |
| `xenonet-mcp` | The MCP servers, on `rmcp` | most of the core, and platforms |
| `xenonet-console` | What only the console has: themes, the Spotify remote, the system panel, opening things, the control channel's client and the steps scripts run; and those cores | api, components, os, text, actors, adapter |

**The composition root.**

| Crate | Its job | Depends on |
| --- | --- | --- |
| `xenonet-service` | `xenonet serve`: makes the cores and the service's own parts (the component service's socket, the assistant's endpoint, the actor gateway, A2A, the network, the terminal drawer's shells), hands them to `xenonet-http` behind its parts' traits (`parts.rs`), and listens | most of the workspace |
| `xenonet-cli` | The `xenonet` binary and every command | everything |

**The composition root** is the cli's `serve()` (`crates/xenonet-cli/src/main.rs`)
with `crates/xenonet-service/src/compose/`. `serve()` calls the service's
`serve::run_with` with a builder that starts from `Cores::pending`, makes
the demo's world first (it moves the folders the rest read), then wires the
map (`xenonet_state::runtime::wire`), the engine and actors
(`xenonet_runtime::wire`, with `compose::connect`'s driver for connecting
your agents and the network's plug), the app's cores
(`compose::app::wire`: opening things, refusals, the decision model, the
system panel, the MCP catalog, the network's hub, remote actors, integrations, the
doctor) and the console's (`xenonet_console::wire`), and puts in the
service's terminals. Whatever made the cores, `serve::wire_root` puts in
the local door (`xenonet_runtime::door`) and the app registry
(`xenonet_platforms::Registry`) before the service starts. Each part gets only the settings it reads, and the
parts reach each other only through the traits in `xenonet-api`. A core
left pending answers its routes with 501 and `{ error, code: "pending" }`,
and a check that can't run refuses. The service's own binary
(`crates/xenonet-service/src/main.rs`, for its tests) runs with every core
pending.

### Dependencies from outside

Each is in for a reason, written beside it in the `Cargo.toml` that brings it in. A new
one needs a reason too.

| Crate | For |
| --- | --- |
| `serde`, `serde_json` (with `preserve_order`), `schemars`, `serde_path_to_error` | Every document, body and record, in the order its keys were added, as the files and golden files have them; the JSON Schemas written from the model; the field a document fails at |
| `rusqlite` (bundled) | `engine.sqlite`, `search.sqlite`, the network's queues, OpenCode's and Codex's databases. Bundled, so FTS5 and `VACUUM INTO` are there whatever the system has |
| `tokio`, `async-trait` | The service's sockets, timers and child processes, and the async core traits |
| `hyper`, `hyper-util`, `http-body-util`, `bytes` | The HTTP API and the gateway, with XenoNet's own router on top, since the order of the checks and routes is part of the contract |
| `rmcp` | The MCP servers over stdio |
| `reqwest`, `rustls` (on ring) | HTTPS to the APIs XenoNet calls, with the system's certificates and proxy |
| `ring`, `x25519-dalek`, `base64` | The network's signatures, sealing, HMAC and hashes, and its keys on disk |
| `sha2`, `getrandom` | Hashes and random values: ids, nonces and the keys made at each start |
| `biscuit-auth` (6) | Actors' grants and their page cursors, in `xenonet-authz` |
| `ruzstd`, `flate2` | Codex's archived rollouts, in Rust alone so no build needs a C compiler; the refusals' gzip context and the component packages |
| `regex`, `url`, `unicode-normalization`, `icu_normalizer` | Patterns without lookaround, URLs as the WHATWG standard reads them, names compared after NFKC and NFD, labels in NFC |
| `portable-pty` | The terminal drawer's shells, in the service (a pty, ConPTY on Windows) |
| `libc`, `windows-sys` | The few system calls std doesn't wrap, and Windows's job objects |

## The layering

The rules, from goal 6. Every crate in `crates/` is in one class:

- **Base crates**: `xenonet-text`, `xenonet-os` and `xenonet-model`, the
  formats, the machine and the shared vocabulary every crate uses. They
  count toward no limit.
- **Core library crates**: every crate not classed below, a new one
  included. Each has at most three internal dependencies besides the base
  crates.
- **Composition crates**: `xenonet-runtime`, `-state`,
  `-assistant`, `-doctor` and `-integrations` each hold a domain's
  runtime and wire several domains together. Each has a ceiling at its
  count when the ceilings were set, with the phase of
  [direction.md](direction.md#order-of-work) it dissolves in (state in
  phase 4, integrations in phase 6, the rest in phase 5).
- **Surfaces**: `xenonet-http` imports `xenonet-api` and the base crates
  and nothing else. `xenonet-mcp` and `xenonet-console` have a ceiling like
  the composition crates', until they reach `xenonet-api` alone in phase 5.
- **The composition root**: `xenonet-service` and `xenonet-cli`, with no
  limit. No other crate depends on either.

And for every crate but the surfaces and the root:

- It never depends on a surface crate or an adapter crate (`adapters/*`,
  `xenonet-platforms` included).
- Only the surfaces, the root and `xenonet-state`, whose runtime drives
  the demo's world, depend on `xenonet-demo`.
- It never names a platform. What it knows of Claude Code, Codex, Grok
  Build, Hermes and the rest comes through `xenonet_adapter::platforms()`,
  the adapters the binary installed at its start
  (`xenonet_platforms::install()`, the first thing each `main` does).
  `xenonet-adapter`, which is the registry, and `xenonet-demo`, which makes
  sessions for named apps, are exempt, and each adapter may name its own
  platform.

`scripts/rust-layers.mjs` checks them, and `npm run lint:rust` runs it
first (in CI's `rust-lint` job too). `node scripts/rust-layers.mjs` fails on
a breach; `--report` lists every breach and each crate's count without
failing. The classes and the ceilings, each with its phase and why, are at
the top of the script. A ceiling only comes down: the check fails when a
crate goes over its ceiling, and when it drops below without the ceiling
being lowered there. A crate the script classes that no longer exists fails
it too.

It reads each crate's `Cargo.toml` for the dependencies (a target's
included, dev-dependencies left out), and each core crate's `src/` for a
registry id in a string literal: an app's id from `apps.json`, or an actor
product's (`hermes-agent`, `grok-bot`, `muse`, `dots`). It leaves out:

- test code: whatever follows a `#[cfg(test)]` in a file, files a test
  module includes by `#[path]`, `tests/`, and dev-dependencies;
- comment lines;
- attribute lines (`#[serde(…)]`, `#[schemars(…)]`) and the model's schema
  macros (`open!`, `tag!`, `constant!`, `pattern!`), where an id is a
  document's wire name written into its schema, not a branch on a platform.

Some words are an app's id and something else too. `NOT_AN_APP`, at the
top of the script, lists each such literal the check lets through: the
file, the exact text on the line and why. They are `"cursor"` as a
page cursor (the MCP catalog, the assistant's reads, the network's relay),
`"pi"` as the Integrations core's method name, and `"codex"` as field names
in `state.json` v1 and in the project detection answer the HTTP golden
files pin. An entry goes there only where the word means something other
than the app, never for a branch on it.

What the check doesn't look at: the adapters' own dependencies
(`grok-build` builds on `claude-code`'s, `hermes` on `xenonet-actors`, and
`xenonet-platforms` on `xenonet-api` for `Registry`), and a surface or the
root naming a platform (the routes' fallbacks name the apps XenoNet asks,
and the service reads Grok Build's config through its adapter for the MCP
catalog).

## The processes

The `xenonet` binary runs as each of these:

- **The service**, `xenonet serve`: one process per instance, on
  127.0.0.1. Desktop starts it standalone (`XENONET_STANDALONE=1`), or
  the system's supervisor does at login ([always-on.md](always-on.md)),
  and it keeps running when Desktop quits.
- **`xenonet mcp`**, `xenonet mcp console` and `xenonet mcp assistant
  --grant <id>`: MCP servers over stdio, each started by the program that
  uses it.
- **`xenonet hook <app>`**: one short process per hook event, for the
  hooks that are commands.
- **The command**: every other `xenonet` command. It calls the core
  directly, or a running XenoNet over its HTTP API (`xenonet_api::client`).
  `xenonet components` is how Desktop installs and updates the Service and
  Tools.

Desktop reaches the service in two ways, and has no IPC of its own:

- the HTTP API on `127.0.0.1:<port>`, as the page does;
- the component service's socket on 127.0.0.1, found through
  `runtime/service-<port>.json`, which holds its port and a token new at
  each start. JSON lines: a hello carrying the token, whose answer carries
  the window key, then `notification` events and `open`, `show` and
  `console` requests from the service, and Desktop's own requests (where
  its window is, the terminals). The tray app connects to the same socket
  as `tray`.

`electron/service-runtime.js` picks the binary: `XENONET_SERVICE_BIN`, the
installed Service's `bin/xenonet`, or in XenoNet Dev the checkout's newer
build of `target/release` and `target/debug`. With none it stops and says
how to build one.

## The service

`xenonet serve` (`crates/xenonet-service`, with the HTTP API in
`crates/xenonet-http`):

- Listens on 127.0.0.1 only: 47840 for installed XenoNet, and each
  instance's own port.
- Checks each request in this order. The Host must be exactly
  `127.0.0.1:<port>` or `localhost:<port>` (421). A request other than GET
  whose Origin isn't XenoNet's own is refused (403). In XenoNet Dev with
  `XENONET_UPSTREAM`, the reads the upstream takes pass to it. A path
  outside `/api/` is a static file from `dist/`. Then, in `routes::api`:
  `/api/svc/` goes to other machines' services before any version check,
  an `x-xenonet-api-version` other than `1` gets 426, and the routes are
  tried in their order, since the first that answers is the one that
  answers.
- Takes JSON bodies, 1 MiB by default and smaller where a route says (413
  over it, 400 when it isn't JSON), and answers errors as
  `{ error, code? }`.
- Has a rule on each route for who may call it, in one table
  (`xenonet-http`'s `routes/rules.rs`): the window key with the process
  check, you, a session's process check, an actor's bearer token, or a
  gateway token.
- Streams the page's events on `GET /api/events` (`retry: 3000`, a ping
  every 25 seconds) and an actor's on `/api/actors/me/stream`.
- Runs the actor gateway on its own port (the API's plus 1010) while it's
  on, with its own checks, MCP over Streamable HTTP without sessions, six
  tools (`whoami`, `peers`, `send`, `inbox`, `report_run`, `heartbeat`),
  A2A at `/a2a`, and its tunnels as child processes.
- Runs the assistant's endpoint ([cloud-assistants.md](cloud-assistants.md))
  and the component service, local sockets with descriptors in the runtime
  folder, JSON lines, hellos, limits and rate limits.
- Keeps the terminal drawer's shells (`terminals.rs`, on `portable-pty`),
  so they live on while Desktop restarts; Desktop only shows them.
- Runs the background work on tokio tasks: the live tailer, the map's
  refresh, the search index, the doctor every half hour, the network's
  link and the gateway's reach checks.

### Reading transcripts

The live tailer has no file watcher. It reads what was added to the files
it follows four times a second, sweeps every 2 seconds, and walks the
folders for new files on every fifteenth sweep, every 30 seconds
(`xenonet-state/src/runtime/live.rs`). Moving this to push is goal 4's.

Each line is parsed whole, as `JSON.parse` reads it (`xenonet_text::json`),
and read through its app's views. Drift is checked on a sample: the 1st,
2nd, 4th, 8th and so on of each kind of line in a file is checked whole
against the vendor schema (`xenonet_schema::vendor::Sampling`). On a 15 MB
Claude Code transcript of 20,000 lines, parsing each line whole took about
48 ms; reading each into a typed view of only the fields the readers use,
borrowing the text, took about 7 ms. That typed reading isn't built.

## The MCP servers

All three are in `crates/xenonet-mcp`, on `rmcp`, over stdio:
`xenonet mcp` (the engine's tools, for sessions and actors), `xenonet mcp
console` (the console operator, through the command's driver of
`xenonet-console`'s control client) and `xenonet mcp assistant --grant
<id>` (a remote actor's read-only tools). Their contract is the golden
files in `crates/xenonet-mcp/tests/golden/`, replayed by
`tests/contract.rs`: the probe, the handshake, `tools/list` with every
schema key for key, and refused calls, in each kind of session.

- **Both protocol eras.** The 2025 handshake (`initialize`, answered with
  the client's version when it's one the server knows, else 2025-11-25),
  or 2026-07-28's `server/discover` with each request's `_meta`.
- **Who's calling.** A Claude Code session is `CLAUDE_CODE_SESSION_ID`. A
  Codex thread comes from the request's `_meta` (`threadId`, or
  `x-codex-turn-metadata.thread_id`), or `CODEX_THREAD_ID`. Each call to
  the service carries `cwd`, `pid` and the `x-xenonet-agent` header, or an
  actor's bearer token, never both. An actor is a program started with
  `XENONET_ACTOR_TOKEN_FILE` or `XENONET_ACTOR_TOKEN` that isn't one of
  the apps' sessions.
- **Process checks.** The server runs as a child of the agent and opens
  its own connections, so the engine's process check can trace it to its
  session. It never daemonizes, which would cut it loose.
- **No XenoNet.** With no XenoNet answering, the doctor tool runs the
  doctor's checks in the server, as `xenonet doctor` does.

### Claude Code channels

Claude Code registers a channel server (one with
`experimental: { "claude/channel": {} }` that sends
`notifications/claude/channel`) only on the 2025 handshake. A server that
answers 2026-07-28 gets "Channel notifications skipped". So for a channel
session, `xenonet mcp` holds to the 2025 handshake:

- **When.** A channel session has `CLAUDE_CODE_SESSION_ID`, and its
  `claude` process (the parent, or `CLAUDE_PID`) was started with
  `--channels` or `--dangerously-load-development-channels` naming
  `server:<this server's name>`. `XENONET_CHANNEL=1` or `0` forces it, as
  on Windows, where XenoNet doesn't read other processes' arguments.
- **The handshake.** `server/discover`, the 2026-07-28 probe, gets
  method-not-found, which sends Claude Code back to `initialize`, where it
  asks for 2025-11-25 and gets it. The server doesn't cap its protocol
  versions to do this: capped at 2025-11-25, `rmcp` answers the probe
  itself with `-32022 Unsupported protocol version` before any handler
  runs. It overrides `discover` instead.
- **What it offers.** `{ tools: {}, experimental: { "claude/channel": {} } }`,
  and the instructions gain the paragraph on channel messages.
- **The messages.** The server long-polls `POST /api/engine/channel`
  (`{waitSeconds: 25, cwd, pid}`, as the session), waits 3 seconds after a
  failure, stops on `{off: true}`, and sends each item as a
  `notifications/claude/channel` notification with
  `params: { content, meta: { from, message_id } }`: `content` as the
  service framed it, `from` with anything but word characters, spaces and
  `.:@-` taken out and cut to 80 characters, and `message_id` as a string.
- **Checked** by `crates/xenonet-mcp/tests/handshake.rs`, which drives the
  server over stdio as Claude Code does, with a fake XenoNet's long poll,
  and by the channel session's golden file.
- **If a later `rmcp` can't be held to the 2025 handshake**, the server
  serves channel sessions over a stdio loop of its own: newline-delimited
  JSON-RPC, `initialize`, `tools/list`, `tools/call` and the notification.

## Hooks

`xenonet hook <app>` reads the hook's JSON on stdin, posts it to
`/api/hook/<app>` with a 4-second timeout, prints the answer on a 200, and
always exits 0, so a XenoNet that's down never stops an agent. An app
XenoNet doesn't tail is refused quietly, with a line in the log.

What `connect` writes into each app's settings comes from its adapter.
Claude Code takes HTTP hooks posting straight to `/api/hook/claude`,
except at `SessionStart`, which takes only commands. Codex and Grok Build
run every hook as a command: `XENONET_PORT=<port> '<bin>' hook <app>`.
The MCP servers are stdio entries running `<bin> mcp` and `<bin> mcp
console`, with `XENONET_PORT` in their environment. `connect` also
recognizes the entries an
earlier XenoNet wrote, which ran Node and `cli/xenonet.js`, as its own, so
connecting again replaces them and `disconnect` takes them out.

Which binary those entries run, and the command on PATH, a local host's
`relay` and a tunnel's adapter (`mcp assistant`), is one rule
(`xenonet_bin` in `crates/xenonet-components/src/runtime.rs`):
`XENONET_BIN` or `XENONET_LAUNCHER_BIN` when set; else the binary in
`XENONET_LAUNCHER_ROOT`; else the running binary. Installed XenoNet's
launcher keeps its own copy of the Tools' binary at
`<runtime>/launcher/xenonet`, which never moves
([contracts.md](contracts.md), section 12).

## Toolchain

`rust-toolchain.toml` pins the latest stable Rust, 1.99.1, with rustfmt and
clippy; rustup installs it for the checkout without changing anyone's
default. It moves to each new stable release once the workspace builds,
tests and lints clean on it. A PATH can name a toolchain's own `cargo`,
which ignores the pin, so the scripts run cargo through
`scripts/cargo.mjs`, which finds the pinned one (`npm run cargo -- …`).

## Static analysis

`npm run lint:rust` (`scripts/rust-lint.mjs`) runs every check, in turn,
and fails if any did. CI runs the same in its `rust-lint` job, and clippy
and the tests on Linux, Windows and macOS in its `rust` job, since clippy
sees each platform's own code only there.

- **Layering.** `scripts/rust-layers.mjs` (see "The layering").
- **Format.** `cargo fmt --check`, with `rustfmt.toml`'s settings.
- **Lints.** `cargo clippy --workspace --all-targets --locked -- -D
  warnings`. The lints are in the root `Cargo.toml`'s
  `[workspace.lints]`, which each crate takes with `[lints] workspace =
  true`: clippy's default groups and `pedantic`, less the pedantic checks
  that are taste rather than bugs, and less the casts and exact float
  comparisons the code makes on purpose to compute as the golden files
  were recorded. On top of those, for a service that runs for days: no
  `unwrap`, `panic!`, `todo!`, `unimplemented!` or `dbg!` outside tests
  (`clippy.toml` lets tests), `expect` only for a real invariant with its
  reason as the message, nothing on stdout but the command's own output
  (stdout is also the MCP servers' protocol), and a `// SAFETY:` comment on
  every `unsafe` block. An exception is `#[expect(lint, reason = "…")]`,
  never `#[allow]`, so one no longer needed shows.
- **Docs.** `cargo doc --workspace --no-deps` with warnings as errors, so
  doc links resolve.
- **Dependencies.** `cargo deny check` (`deny.toml`): no RustSec advisory,
  licenses that fit MIT, no wildcard versions, crates only from crates.io.
  `cargo shear`: no dependency a crate doesn't use.
  `npm run lint:rust -- --fix` runs `cargo fmt` and `cargo shear --fix`
  first.

`unsafe` is denied (`unsafe_code = "deny"`) except where a function says
why with `#[expect(unsafe_code, reason = …)]`. It is all calls to the
system that std doesn't wrap: libc (`localtime_r`, `access`, `sysctlbyname`,
`kill`) and the Mach and Windows APIs for processes, CPU and memory, mostly
in `xenonet-os` and `xenonet-engine`'s `procs_native.rs`. Miri can't run
those, so the tests exercise them on each platform in CI instead.

Locks go through `xenonet_os::sync` (`locked()`, `reading()`,
`writing()`) rather than `lock().unwrap()`. std poisons a lock whose holder
panicked, and every later `unwrap` would panic too, taking the service down
with one bad request. The state under XenoNet's locks is plain data that
each call leaves whole, so a later call takes the lock as it is.

## Building for each platform

`scripts/rust-release.mjs` builds the binary for a release, for one
platform and CPU, with Cargo's `dist` profile: whole-program LTO in one
codegen unit, no debug info and no symbol table. On an M-series Mac that
made the service about a third smaller than the `release` profile (34.5 MB
to 22.1 MB) and 5-10% quicker to scan and search, for a build about three
times as long, so local builds stay on `release`.

| Target | Built on |
| --- | --- |
| `aarch64-apple-darwin`, `x86_64-apple-darwin` | A Mac, both from one machine |
| `x86_64-unknown-linux-gnu` | Linux (the release's `headless` job, on `ubuntu-latest`) |
| `aarch64-unknown-linux-gnu` | Linux; `rust-release.mjs` knows the target, but the release workflow doesn't build it |
| `x86_64-pc-windows-msvc` | Windows |

- The binary is the Service and Tools packages, as `bin/xenonet`
  (`xenonet.exe`), with the schemas it reads and, for the Service, the
  built page, and no Node runtime or JS beside it
  (`scripts/component-build.mjs`). A package's startup check runs
  `bin/xenonet --version --json`. The node launcher script that entries an
  earlier XenoNet wrote still run is built into the binary
  (`xenonet-components`' `launcher.cjs`).
- **macOS signing.** The binary is ad hoc signed (`codesign --sign -`), as
  the app is, after Xcode's `strip`. The build stops if a certificate
  signs the app or the package, because a certificate carries its owner's
  name; `XENONET_SIGN=1` signs on purpose, with a Developer ID.
- **No home folder in the binary.** The build remaps the cargo registry,
  the checkout, rustup's folder and the home folder
  (`--remap-path-prefix`), and fails if the binary still holds the home
  folder's path.
- SQLite is compiled in, so no platform needs its own.

## What stays compatible

Nothing about the wire or the disk changes without a reason. A golden
file changes only when the behaviour should ([AGENTS.md](../../AGENTS.md)),
and backwards compatibility is a non-goal ([goals.md](goals.md)), so this
lists what XenoNet holds to now, not a promise for every release.

- **HTTP.** Everything in "The service": routes, methods, checks, limits,
  status codes, error bodies and event streams, as the golden files in
  `crates/xenonet-cli/tests/golden/` pin them.
- **Hooks and MCP.** The endpoints, payloads, tool names and schemas, as
  `crates/xenonet-mcp/tests/golden/` pins them; and an earlier XenoNet's
  entries in the apps' settings are still recognized as XenoNet's.
- **Tokens and ids.** `xenonet_actor_` and `xenonet_gateway_` followed by
  the base64url of a Biscuit v6 token rooted at the actor's machine;
  XenoNet keeps only their revocation ids ([authorization.md](authorization.md),
  "The actor grant"). The local API's credential, the window key and the
  component service's token (new at each start), the assistant's page
  cursors (its grant narrowed to one read), and every id format in
  `xenonet-model`'s `ids`.
- **SQLite.** `engine.sqlite` (WAL, foreign keys, `user_version` 8) and
  `search.sqlite` (FTS5 with `porter unicode61 remove_diacritics 2`, its
  own `user_version`). A migration continues the numbered steps
  (`xenonet-blocks`' `sqlite.rs`); a database about to be migrated is
  copied first (`VACUUM INTO`) and the copy deleted a week later; a
  database a newer XenoNet moved further is left at its version.
- **State files.** `state.json` (version 2, with the step from 1),
  `drift.json`, `doctor.json`, the delivery
  policy and its audit log, the logs (`log/entry/v1`), the refusals, the
  assistant's credentials and tombstones, and the runtime descriptors.
  Each is written through a temporary file renamed into place
  (`xenonet-blocks`' `store.rs`). A file from a newer XenoNet is read and
  never written back; one XenoNet can't use is moved aside to
  `<name>.broken-<time>.json`, never written over; an older one is kept as
  `<name>.v<n>.json` before it's migrated.
- **Secrets** stay in the operating system's credential store
  (`xenonet-actors`' `secret_store.rs`: `security` on macOS, the Windows
  credential manager, `secret-tool` on Linux), never in a file.
- **Schemas.** `schemas/` is the source of truth for what goes over the
  wire and into files: XenoNet's own written from `xenonet-model`, the
  vendors' hand-kept ([schemas.md](schemas.md)).
- **Desktop and older Services.** Desktop still runs a Service from before
  0.4.0, with its own node, until it can be replaced
  (`electron/service-runtime.js`).

The service is the only writer of `state.json` and `engine.sqlite`.
