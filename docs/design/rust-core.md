# Rust core

A design for moving everything in XenoNet except Electron and the console
page to Rust: the core, the transcript readers, the HTTP API, the gateway
and actor gateway, the MCP servers, the `xenonet` command and the drift
tooling. The port is under way. The last section says which modules are
done.

**In short:**

- One cargo workspace at the repository's root, with a crate per layer of
  today's `core/`, `server/`, `mcp/` and `cli/`. They build one binary,
  `xenonet`, that serves the API, runs the MCP servers and hooks, and is
  the command.
- While the port runs, JS reaches Rust in process, through a Node-API
  addon built with napi-rs. Each ported module gets a Rust-backed twin
  with the same exports. A resolve hook loads the twin in place of the JS
  module, so the unchanged JS test suite runs against Rust. A module's JS
  goes only when every test that reaches it passes that way.
- At the end, the processes stay as they are today: the service is its own
  process, and Claude Code and Codex start `xenonet mcp` and `xenonet hook`
  themselves. Each becomes the Rust binary. Electron talks to the service
  through the HTTP API and the component service's socket, both of which
  exist already. No new IPC protocol.
- Everything on the wire and on disk stays as it is: routes, hooks, tool
  names and schemas, tokens, SQLite files and state files. A Rust service
  reads what a JS one wrote, and the other way round, so the two can run
  side by side while the port runs.
- The MCP servers use the official `rmcp` crate. A Claude Code channel
  session still gets the 2025 handshake (protocol 2025-11-25), which is
  the only one on which Claude Code registers a channel server.

## Why

- Claude Code and Codex start `xenonet hook` on every hook event and
  `xenonet mcp` for every session. Each is a Node process today. On a
  recent Mac, a Codex hook run through Node takes about 88 ms from start
  to exit, with XenoNet's answer; the Rust `xenonet hook` takes about
  3 ms. Codex runs each of its hooks as a command, so a tool call starts
  two.
- The service, Tools and a host become one file per platform, with no
  Node runtime, launcher script or `ELECTRON_RUN_AS_NODE` beside them.
- The service runs all the time (see [Always-on XenoNet](always-on.md)).
  Its tailer, scanner and search index read every transcript on the
  machine, so their memory and CPU matter more than they did.
- The contracts in `schemas/` become types that the compiler checks,
  instead of conventions the tests check.

## What stays in JS

- `electron/`: the desktop app, its window, tray, terminals and control
  channel.
- `src/`: the console page, with `core/units.js` and `core/themes.js`,
  which it imports. The Rust command has its own copy of the units it
  prints.
- `test/e2e/` and `test/real/`, which drive the app from outside.
- The build and development scripts in `scripts/` other than the drift
  check: `dist.mjs`, the builder hooks, the lab, the benchmarks.

## The workspace

```
Cargo.toml                 the workspace: shared versions, the release profile
crates/
  xenonet-base             paths.js, pricing.js, dirs.js, log.js, schema/ids.js, JSON and time as JS reads them,
                           units.js for the command, private-file.js, client.js
  xenonet-blocks           core/blocks/: lifecycle, scanner, gatherer, tailer, normalizer, JSON and SQLite stores
  xenonet-schema           core/schema/, vendor.js, drift.js: the schemas, lenient reading, drift records
  xenonet-sources          lines.js, claude.js, codex.js, git.js, wiki.js, live.js, numbat.js, demo.js, native/,
                           claude-usage.js, openrouter-usage.js, reported-runs.js, xenolith.js, refusals/
  xenonet-state            graph.js, state.js, search.js, digest.js, stats.js, identities.js, projects.js,
                           workspace.js, classify.js, store.js, statefile.js, migrate.js, redact.js, frames.js,
                           rename.js, export-labels.js, needs-you.js
  xenonet-engine           engine/, delivery/, procs.js
  xenonet-actors           actors/, assistant/, secret-store.js, integrations/
  xenonet-hosts            hosts/
  xenonet-doctor           doctor/, diagnostics.js
  xenonet-server           server/: the API, static files, upstream, the actor gateway and its tunnels,
                           the assistant endpoint, the component service
  xenonet-mcp              mcp/: the xenonet, xenonet-console and assistant servers
  xenonet-cli              cli/: the xenonet binary and every command
  xenonet-drift            scripts/provider-drift*: the check of new provider releases
  xenonet-node             the Node-API addon behind the JS twins, for the port
xenolith/rust/crates/      Xenolith's crates (xenolith-core and its command), which join the workspace
```

Each crate depends only on crates above it in the list, as the layers of
`core/` do today. refusals/ sits with the sources, not the actors, since
the live tailer reads every line through refusals/detect.js. `server`, `mcp` and `cli` are thin layers over the rest
and don't import each other, except that `xenonet-cli` starts the other
two. Only `xenonet-node` knows about Node.

The workspace uses cargo's resolver 3, which picks dependency versions
that build on the declared `rust-version`. The crates are edition 2021
and need Rust 1.85. Xenolith's are edition 2024; editions mix per crate.

### Dependencies

The crates the port uses, each for a reason. `serde_json`, the napi
crates, `rusqlite`, `sha2`, `getrandom` and (in the MCP crate) `rmcp` and
`tokio` are in so far. The others come with the crates that need them.

| Crate | For |
|---|---|
| `serde`, `serde_json` (with `preserve_order`) | Every JSON file, body and record. JS keeps an object's keys in the order they were added, and so do the files XenoNet writes, so the Rust side keeps that order too. |
| `napi`, `napi-derive`, `napi-build` | The addon. Version 3.4, the last that builds on Rust 1.85. |
| `rusqlite` (bundled) | `engine.sqlite` and `search.sqlite`. The bundled SQLite has FTS5 and `VACUUM INTO`, and doesn't depend on the system's version. |
| `tokio` | The service's sockets, timers and child processes. |
| `hyper`, `hyper-util` | The HTTP API and the actor gateway. XenoNet's own router on top, because the order in which `server/index.js` checks the Host, the origin, the version header and then each route is part of the contract, and a framework's router would reorder it. |
| `rmcp` | The MCP servers, stdio and Streamable HTTP. |
| `sha2`, `hmac`, `getrandom` | Tokens and their hashes, the assistant's page cursors and attention ids. |
| `ruzstd`, `flate2` | Codex's archived rollouts (`.jsonl.zst`), with a decoder in Rust alone so no build needs a C compiler for it, and the refusals' gzip context. |
| `notify` | The recursive folder watches the tailer and the themes folder use. |
| `regex` | Where the JS uses a regular expression without lookaround. Lookarounds are written out by hand. |

## How JS reaches Rust

### During the port: napi-rs, behind each module's own API

The JS unit tests are the contract: 155 files and 1,288 tests,
which import JS modules, call them, often synchronously, pass them fakes
(a clock, a filesystem, callbacks such as a scanner's `pick` and `look`),
and read their fields (`scanner.folders`, `normalizer.counters`). Rust
must pass them before it replaces any JS. So:

- **The addon.** `crates/xenonet-node` is a `cdylib` built with napi-rs.
  `npm run rust:build` builds it and puts it at
  `crates/xenonet-node/xenonet.<platform>-<arch>.node`. Node-API is stable
  across Node and Electron versions, so the same file loads in both.
- **Twins.** For each ported module, `crates/xenonet-node/js/core/<path>.js`
  is a JS module with the same exports as `core/<path>.js`, backed by the
  addon. For `core/blocks/scanner.js` it's
  `crates/xenonet-node/js/core/blocks/scanner.js`. A twin is the module's
  API and nothing more: classes, getters, the EventEmitter a block is, and
  argument defaults. The logic is Rust's.
- **The hook.** `crates/xenonet-node/js/register.js` registers a Node
  resolve hook (`module.register`). Run with
  `node --import ./crates/xenonet-node/js/register.js`, every import of a
  `core/` module that has a twin loads the twin instead, from tests and
  from the JS that imports it. The twin's own imports aren't redirected,
  so a twin can import JS that isn't ported yet.
- **Parity.** `crates/xenonet-node/parity.json` lists the ported modules
  and the test files that pass with every twin in place. `npm run
  test:rust` runs those files with the hook, and `npm run test:rust --
  --all` runs the whole suite that way and says which files pass, to find
  the next. `npm test` runs the JS until a twin moves into place.

What crosses the boundary, and how:

- **Values** convert through `serde_json`, with their keys in order.
  Records the readers hand around are Rust objects with getters (a
  transcript line is parsed once, in Rust, and read where it's needed).
  Numbers, `__proto__` keys and dates convert as JS would make them
  (`crates/xenonet-node/src/js.rs`), not as napi's own conversion does.
- **Time.** Rust cores take the time as an argument, or a `Clock`. A twin
  passes its clock's `now()`, so a test's hand-moved clock moves Rust
  too. Timers belong to whoever runs the core: tokio in the binary, the
  JS clock's `setTimeout` in a twin.
- **Files** go through an `Fs` trait. It's `std::fs`, except in a twin
  that a test gave a fake `fs`, where Rust calls the fake back during the
  call. Rust lists a folder in Node's order, which is by name, byte for
  byte, on unix (libuv sorts what `scandir` finds), and the filesystem's
  own order on Windows.
- **Callbacks** that are synchronous (`pick`, `enter`, `normalize`,
  `inspect`) are called from Rust during the call. Asynchronous work (a
  breadth-first walk whose `look` awaits) is a Rust state machine that
  says what to read next, and whoever runs it does the reading: tokio in
  the binary, `fs.promises` in a twin. The decisions the tests check are
  Rust's, and the JS around them only reads files.
- **Machines.** Where the JS awaits in the middle of its decisions (the
  process lookups, the engine's checks of who sent a request, a store or
  a command a test fakes), the Rust is plain code against a trait for what
  it reads. Behind a twin it runs on a thread of its own
  (`crates/xenonet-node/src/machine.rs`): each time it needs something it
  asks and waits, and the twin answers, awaiting what it must
  (`js/drive.js`). JS waits only while Rust computes between questions.
  An answer that isn't a promise goes back at once and one that is costs
  one await, as the JS's own call or await did, so other work runs between
  the same steps it ran between in the JS. The collaborators a module was
  given (the actors, a store, a test's fake) are called by name when the
  JS called them (`xenonet-engine/src/peer.rs`), so what stands in their
  place then is what's called.
  In the binary the same code runs against the system itself.
- **Events.** A block's EventEmitter stays in its twin. Rust returns each
  state change, and the twin emits it.
- **Tasks.** An async module whose awaits nest deeply (control.js's
  launch, a host's relay) is ported as async Rust: wherever the JS awaited
  a fetch, a child process, a timer or a function it was given, the Rust
  awaits `io.ask(request)` (`crates/xenonet-app/src/task.rs`). The twin
  steps the task, does what it asks with the fakes or Node's own calls,
  and hands back each reply (`crates/xenonet-node/js/task.js`). The logic
  ports await for await, and in the binary tokio does the asking.

A call into the addon costs about 20 ns. A call back into JS costs more,
about 300 ns with a string to pass, which matters only where a twin calls
back for each of many items, and only during the port: the binary has no
callbacks into JS.

A module's JS is replaced once every test file that reaches it passes
with its twin, and its known differences are written in `parity.json`.
The twin then moves into place (`core/<path>.js` re-exports it), and the
JS service runs on Rust for that module. When nothing in JS imports the
module any more, its tests move to Rust, test for test, and the twin
goes.

### At the end: the processes XenoNet already has

The addon is for the port only. The finished XenoNet doesn't need it,
because the processes that matter are already separate:

- **The service** is its own process. Desktop starts it today with
  Electron's `utilityProcess.fork`. In the always-on design it runs
  standalone (`XENONET_STANDALONE=1`) and Desktop connects to it. It
  becomes `xenonet serve`.
- **The hooks and MCP servers** are processes Claude Code and Codex start.
  They become `xenonet hook <app>` and `xenonet mcp`.
- **A host** is reached over SSH with `xenonet relay`, which becomes the
  same binary.

So Electron talks to the Rust service the way it talks to the standalone
service now: the HTTP API on 127.0.0.1, and the component service's
socket, found through `runtime/service-<port>.json`, with JSON lines,
a hello carrying the token, `notification` events, and `open` and
`console` requests. A Rust binary can't be a `utilityProcess` child, so
the parent-port messages Desktop gets today (`window-key`, `ready`,
`error`, `notify`, `open`, and the development control relay) move onto
that socket, where the service's answer to the desktop's hello already
carries the window key.
That's the one change the port makes to `electron/`, and the always-on
design makes the same one.

The other option for the port was a sidecar, with JS calling a Rust
process over stdio for each module. It fails the tests in three ways. A
synchronous call would block the event loop on a pipe. A fake in the
test's process can't be called back from another process. And every
module would need its own RPC wrapper. A sidecar would also leave the JS
service with no Rust to use until all of it was ported.

## The service

`xenonet serve` replaces `node server/index.js`:

- The same ports (47840, and each instance's own), on 127.0.0.1 only.
- The same checks, in the same order: the Host must be exactly
  `127.0.0.1:<port>` or `localhost:<port>` (421); a request other than GET
  with an Origin that isn't XenoNet's own is refused (403); an
  `x-xenonet-api-version` other than `1` gets 426; bodies are JSON,
  1 MiB by default and smaller where a route says, 413 over it and 400
  when it isn't JSON; errors are `{error, code?}`.
- Every route, in the order `api()` dispatches them, with each route's
  rule for who may call it: the window key with the process check, the
  person, a session's process check, an actor's bearer token, a gateway
  token. The routes the open pull requests add come with them.
- The page's event stream, `GET /api/events`: `retry: 3000`, a ping every
  25 seconds, and the same event names and data. And an actor's
  `/api/actors/me/stream`.
- The actor gateway on its own port (the API's plus 1010) while it's on,
  with its own checks in their order, MCP over Streamable HTTP without
  sessions, and its six tools. Its tunnels are child processes, as now.
- The assistant endpoint and the component service, local sockets with
  descriptors in the runtime folder, JSON lines, the same hellos, limits
  and rate limits.
- The background work the service does now (the tailer's watch and
  sweep, the refreshes, the search index, the doctor every half hour, the
  gateway's reach checks), on tokio tasks.

The JS route tests (`server.test.js`, `engine-routes.test.js`,
`actor-routes.test.js` and the rest) start a server and talk HTTP to it.
They get a switch that starts `xenonet serve` instead, so the same files
check the binary.

### Reading transcripts fast

Measured on a 15 MB Claude Code transcript of 20,000 lines, on a recent
Mac:

| Reading | Time |
|---|---|
| The JS readers (`JSON.parse` and the views' getters) | 40 ms |
| Rust, each line parsed into a whole JSON value (XenoNet's parser, as `JSON.parse` reads) | 48 ms |
| Rust, the same with serde_json's own parser | 38 ms |
| Rust, each line read into a typed view of only the fields the readers use (serde, borrowing the text) | 7 ms |

A whole JSON value per line is no faster than V8's `JSON.parse`. So the
Rust readers read each line into a typed view of the fields they read,
leniently (a field of another type reads as missing, as the vendor check
leaves it), and build the whole record only for a line the drift check
looks at in full: the first of each kind in a file, then the 2nd, 4th,
8th and so on. The JSON reader that reads as `JSON.parse` does stays for
the stores and for those lines.

## The MCP servers

All three move to `rmcp`, with stdio: `xenonet mcp` (the engine's tools),
`xenonet mcp console` and `xenonet mcp assistant --grant <id>`.

- **Tools.** The same names, titles, descriptions, annotations and input
  schemas. The JS servers declare theirs with zod. A contract test takes
  each server's `tools/list` from the JS server and from the Rust one and
  compares them. The assistant's tools keep their output schemas and
  strict inputs.
- **Both protocol eras**, as `serveStdio` does today: the 2025 handshake
  (`initialize`, answered with the client's version when it's one the
  server knows, else 2025-11-25), or 2026-07-28's `server/discover` with
  each request's `_meta`.
- **Who's calling.** A Claude Code session is `CLAUDE_CODE_SESSION_ID`. A
  Codex thread comes from the request's `_meta` (`threadId`, or
  `x-codex-turn-metadata.thread_id`), or `CODEX_THREAD_ID`. `rmcp` hands
  a tool the request's `_meta`. Each call carries `cwd` and `pid` and the
  `x-xenonet-agent` header, or an actor's bearer token, as now.
- **Process checks** (`core/engine/verify.js`) hold unchanged: the binary
  runs as a child of Claude Code or Codex, as the Node process did, and
  opens its own connections. It doesn't daemonize, which would cut it
  loose from the agent and fail the check.

### Claude Code channels

Claude Code registers a channel server (one with
`experimental: { "claude/channel": {} }` that sends
`notifications/claude/channel`) only on the 2025 handshake. A server that
answers 2026-07-28 gets "Channel notifications skipped". The JS server
(`mcp/xenonet.js` on the live delivery branch) handles this by serving a
channel session over a stdio transport wired by hand, which speaks only
the 2025 handshake. The Rust server keeps that behavior:

- **When.** A channel session is one with `CLAUDE_CODE_SESSION_ID` whose
  `claude` process (the parent, or `CLAUDE_PID`) was started with
  `--channels` or `--dangerously-load-development-channels` naming
  `server:<this server's name>`. `XENONET_CHANNEL=1` or `0` forces it.
- **The handshake.** For a channel session, `server/discover` (the
  2026-07-28 probe) gets method-not-found, as the JS server answers it,
  which sends Claude Code back to `initialize`, where it asks for
  2025-11-25 and gets it. The server doesn't cap its protocol versions to
  do this: capped at 2025-11-25, rmcp answers the probe itself with
  `-32022 Unsupported protocol version` before any handler runs, which
  isn't the answer known to work. So it overrides `discover` instead.
- **What it offers.** `{ tools: {}, experimental: { "claude/channel": {} } }`,
  and the instructions gain the paragraph on channel messages.
- **The messages.** The server long-polls `POST /api/engine/channel`
  (`{waitSeconds: 25, cwd, pid}`, as the session), waits 3 seconds after
  a failure, stops on `{off: true}`, and sends each item as a
  `notifications/claude/channel` notification with
  `params: { content, meta: { from, message_id } }`: `content` as the
  service framed it, `from` with anything but word characters, spaces and
  `.:@-` taken out and cut to 80 characters, and `message_id` as a
  string.
- **Checked.** `crates/xenonet-mcp` is a working prototype on rmcp 3.5.
  Its tests drive it over stdio as Claude Code does (the probe, the
  handshake, the tools, a message from a fake XenoNet's long poll), and
  the same sequence against the JS servers gives the same answers in a
  channel session and in any other: the probe's code, the protocol
  version, the capabilities and the tools. Claude Code itself hasn't run
  against it yet.
- **The fallback.** If a later `rmcp` can't be held to the 2025
  handshake, the server serves channel sessions over its own stdio loop,
  as the JS server does: newline-delimited JSON-RPC, `initialize`,
  `tools/list`, `tools/call` and the notification.

### Toolchain

`rust-toolchain.toml` pins the latest stable Rust, 1.99.1 (rustup installs
it for the checkout without changing anyone's default), so builds are the
same everywhere and clippy has its latest checks. It moves to each new
stable release once the workspace builds, tests and lints clean on it.
Every current `rmcp` release needs Rust 1.88 or later (through let-chains
and `darling` 0.23), so `crates/xenonet-mcp` builds into the one binary
with the rest. The crates that hold `rmcp` or `tokio` (`xenonet-mcp`,
`xenonet-service`, `xenonet-cli`) declare Rust 1.88; the rest still build
on 1.85. A PATH can name a toolchain's own `cargo` rather than rustup's,
which ignores the pin, so the scripts run cargo through
`scripts/cargo.mjs`, which finds the pinned toolchain.

### Static analysis

`npm run lint:rust` runs what every change to the Rust core passes, in CI
as well (`scripts/rust-lint.mjs`):

- **Format.** `cargo fmt --check`, with `rustfmt.toml`'s settings.
- **Lints.** `cargo clippy --workspace --all-targets -- -D warnings`. The
  lints are in the root `Cargo.toml`'s `[workspace.lints]`, which each
  crate takes with `[lints] workspace = true`: clippy's default groups and
  `pedantic`, less the pedantic checks that are taste rather than bugs, and
  less the casts and exact float comparisons the port makes on purpose to
  compute as the JS does. On top of those, for a service that runs for
  days: no `unwrap`, `panic!`, `todo!`, `unimplemented!` or `dbg!` outside
  tests (`expect` stays for a real invariant, its message saying why),
  nothing on stdout but the command's own output, since stdout is also the
  MCP servers' protocol, and a `// SAFETY:` comment on every `unsafe`
  block. `unsafe` is denied except in a crate or module that says why.
  An exception is `#[expect(lint, reason = "…")]`, never `#[allow]`, so one
  no longer needed shows.
- **Docs.** `cargo doc --no-deps` with warnings as errors, so doc links
  resolve.
- **Dependencies.** `cargo deny check` (`deny.toml`): no RustSec advisory
  (vulnerable, unsound, unmaintained or yanked), licenses that fit MIT, no
  wildcard versions, crates only from crates.io. `cargo shear`: no
  dependency a crate doesn't use.

Locks go through `xenonet_base::sync` (`locked()`, `reading()`,
`writing()`) rather than `lock().unwrap()`. std poisons a lock whose
holder panicked, and every later `unwrap` would panic too, taking the
service down with one bad request; the state under XenoNet's locks is
plain data each call leaves whole, as the JS did when a request threw.

Miri doesn't apply: the core's `unsafe` is FFI (Node-API in
`xenonet-node`, a few libc calls), which Miri can't run. The JS tests
exercise it instead, against the twins (`test:rust`) and the binary
(`test:bin`), on macOS, Linux and Windows.

## Hooks

`xenonet hook <app>` stays what it is: it reads the hook's JSON on stdin,
posts it to `/api/hook/<app>` with a 4-second timeout, prints the answer
on a 200, and always exits 0. Claude Code's HTTP hooks don't run the
command at all and don't change.

What changes is the command XenoNet writes into Claude Code's and Codex's
settings and MCP lists: the binary instead of `'<node>' '<entry>'`, and
no `ELECTRON_RUN_AS_NODE` (`XENONET_PORT=<port> '<bin>' hook <app>`, and
`<bin> mcp`). `connect` recognizes both forms as its own, so connecting
again replaces the old entries, and `disconnect` takes out either. Until
then the preview shows the old entries as different, with a note, and
the doctor says to connect again.

The same goes for everything else XenoNet writes into other programs or
starts for you: the `xenonet` command install puts on PATH, a local
host's `relay`, and tunnel-client's adapter (`mcp assistant`). Which
binary each runs is one rule (`xenonetBin` in
`core/components/runtime.js`, `xenonet_bin` in
`xenonet-components/src/runtime.rs`): `XENONET_BIN` when it's set;
installed XenoNet's launcher, `<state>/runtime/launcher/xenonet`, a copy
of the Tools' binary that never moves (docs/design/contracts.md, section
12); else the running binary, or the checkout's build for the JS.

## What stays compatible

Sessions, actors and tokens that exist when a machine moves to the Rust
XenoNet keep working, and nothing has to be set up again:

- **HTTP.** Everything in "The service" above: routes, methods, checks,
  limits, status codes, error bodies, event streams.
- **Hooks and MCP.** The same endpoints, payloads, tool names and
  schemas. Only the command line in the apps' settings changes.
- **Tokens and ids.** `xenonet_actor_`, `xenonet_gateway_` and
  `xenonet_assistant_` followed by 64 hex digits; XenoNet keeps only their
  SHA-256 in hex. The window key and the service token (64 hex digits,
  new at each start), the assistant's page cursors (base64url JSON and an
  HMAC-SHA256), and every id format in `core/schema/ids.js`. A token the
  JS XenoNet issued is accepted by the Rust one.
- **SQLite.** `engine.sqlite` (WAL, foreign keys, `user_version` 7 today,
  a `VACUUM INTO` copy before a migration, kept a week, and the same
  pruning) and `search.sqlite` (FTS5 with `porter unicode61
  remove_diacritics 2`). The Rust stores open the same files. Their
  migrations continue the same numbered list, so the next step is 8
  whichever side writes it, and a database a newer XenoNet moved further
  is left at its version.
- **State files.** `state.json` (version 2), `hosts.json`
  (`host/list/v1`), `drift.json`, `doctor.json`, the delivery policy and
  its audit log, the logs (`log/entry/v1`), the refusals, the assistant's
  credentials and tombstones, and the runtime descriptors. The same
  shapes, key order and indentation, the same writes through a temporary
  file renamed into place, the same file modes where the JS sets one, and
  the same rules: a file from a newer XenoNet is read and never written
  back, and one XenoNet can't use is moved aside, never written over.
- **Secrets** stay in the operating system's credential store, under the
  same names, through the same tools (`security`, the Windows credential
  manager, `secret-tool`).
- **Folders and settings.** The state, cache and runtime folders resolve
  as `core/dirs.js` does, and every `XENONET_*` variable keeps its
  meaning.
- **Schemas.** `schemas/` stays the source of truth for what goes over the
  wire and into files. The Rust types are checked against it, as
  `npm run contracts` checks the JS.

Since both sides read and write the same files the same way, a Rust
service with a JS command, or the other way round, works during the
port. The service stays the only writer of `state.json` and
`engine.sqlite`, whichever language it's in.

## Building for each platform

| Target | Built on |
|---|---|
| `aarch64-apple-darwin`, `x86_64-apple-darwin` | A Mac, both from one machine, one per app as today |
| `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` | Linux, in a container with an older glibc, so the binary runs on older systems too |
| `x86_64-pc-windows-msvc` | Windows |

- The binary and the addon go where the JS they replace went: in the
  app's resources, and in the Service and Tools packages as `bin/xenonet`
  (`xenonet.exe`). The command on your PATH and the launcher point at the
  binary instead of Node and an entry script.
- **macOS signing.** Builds are ad hoc signed, as the app is now:
  `codesign --sign -` on the binary and the addon in the build hooks. The
  build already stops if a certificate signs the app or the package,
  because a certificate carries its owner's name. It checks the binary and
  the addon the same way, and `XENONET_SIGN=1` still signs on purpose,
  with a Developer ID.
- **No home folder in the binaries.** A Rust binary records source paths
  (in panic messages and debug info), including the cargo registry's
  under the home folder. Release builds remap them
  (`--remap-path-prefix`) and strip debug info, and the build's existing
  check for the home folder's path in packaged files covers the binaries.
- **The dist profile.** Published binaries build with Cargo's `dist`
  profile: whole-program LTO in one codegen unit, no debug info and no
  symbol table. On an M-series Mac that took the service from 34.5 MB to
  22.1 MB and made scanning and search 5-10% quicker than the `release`
  profile, for a build about three times as long.
- SQLite is compiled in, so no platform needs its own.
- CI builds every target, runs `cargo test` and the parity tests on each,
  and attaches the binaries to the release with the rest of its files.

## The order of the port

1. **What the open pull requests don't touch.** Paths,
   pricing and time; the blocks (lifecycle, normalizer, scanner with the
   fix that reads a folder again when it was read within 2 seconds of its
   mtime, tailer with the fix that catches up known files as live after a
   restart, gatherer, stores); the transcript lines. Each behind its twin.
2. **Readers and schemas.** Lenient reading, the vendor readers and drift
   (once the drift work for every provider has landed), the Claude Code
   and Codex readers, the live tailer, stats and search.
3. **The engine.** Engine, delivery, scopes and guards, once their pull
   requests have landed, then actors and tokens, the assistant, and
   hosts.
4. **The binary.** `xenonet hook` and `xenonet mcp` first, since they
   gain the most from starting natively. Then the other commands. Then
   `xenonet serve`, with the route tests run against it.
5. **The switch.** Desktop starts the binary as the standalone service,
   and only the binary: there's no JS service to fall back to. Then the
   JS core and the addon go, and the remaining tests move to `cargo test`.

Each step leaves `npm test` passing on JS, and adds to `parity.json`.

## Decided along the way

- **The toolchain** is pinned with the MCP crate (see "Toolchain").
- **`hyper` with XenoNet's own router**: `crates/xenonet-service/src/routes`
  dispatches in the order `api()` does, through every check in its place.
- **The cores behind traits.** The service's routes call each core
  through a trait shaped after the JS object `server/index.js` hands them
  (`crates/xenonet-service/src/cores`: `State`, `Engine`, `Verify`,
  `Actors`…), JSON in and out. A core not in Rust yet answers 501 with
  `{ error, code: "pending" }`, and a check that can't run refuses, until
  a driver puts the real core in its place.
- **The binary drives the cores itself.** The slices' Rust decides, and
  each twin's JS did the waiting around it: processes, timers, fetch,
  watches, events. In the binary, crates of drivers do that natively, on
  tokio: `xenonet-wire-map` (the state, the readers, the store, the demo's
  world, the schema checks and drift), `xenonet-wire-engine` (the engine,
  delivery, verify and the actors) and `xenonet-wire-app` (hosts and the
  hub, the assistant, the doctor, integrations and the app's own parts).
  `xenonet serve` runs them in that order, the map first, since the demo's
  world moves the folders the rest read; they reach each other only
  through the traits. A core's events go through an `Emitter` to
  `server/index.js`'s handlers (`glue.rs`), fetch through one HTTPS client
  (`net.rs`: reqwest on rustls, with the system's certificates), and a
  task of the hosts slice runs on a thread of its own, its requests
  answered on the service's runtime. The command's drivers
  (`crates/xenonet-cli/src/cores`) do the same for the commands.
- **Terminals live in the service**, as the standalone JS service's do:
  `crates/xenonet-service/src/terminals.rs`, on `portable-pty` (a pty, and
  ConPTY on Windows, as node-pty is for the JS). The service owns the
  shells and Desktop only reads their output, so they keep running while
  Desktop restarts, which a pty in Electron couldn't do.
- **The console operator is in Rust too**: `xenonet mcp console`
  (`crates/xenonet-mcp/src/console.rs`) declares `mcp/console.js`'s tools
  and runs each through the command's driver of `core/control.js`, the
  one `xenonet console`, `script` and `film` use. `core/control.js` itself
  stays JS for `electron/`, the end-to-end tests and the scripts.

## Not decided yet

- Signing on Windows.
- A file watcher for the binary's readers. They poll for now, with no
  new crate: the tailer every 250 ms, a sweep for new and woken
  transcripts every 2 seconds, the shared context's and themes' folders
  every second or half second.
- Whether a release ships Rust subsystems inside the JS service (twins in
  place) before the binary replaces the service.

## Status

`crates/xenonet-node/parity.json` is the record: the modules ported, what
differs from their JS, and the test files that pass with every twin in
place. This table follows it.

| Module | Rust | Its own tests, with twins |
|---|---|---|
| `core/pricing.js` | `xenonet-base/src/pricing.rs` | `costs`, `stats` pass |
| `core/paths.js` | `xenonet-base/src/paths.rs` | `platform`, `windows-command` pass |
| `core/blocks/lifecycle.js` | `xenonet-blocks/src/lifecycle.rs` | `blocks-lifecycle` passes |
| `core/blocks/normalizer.js` | `xenonet-blocks/src/normalizer.rs` | `blocks-normalizer` passes |
| `core/blocks/scanner.js` | `xenonet-blocks/src/scanner.rs`, with the read-again fix | `blocks-scanner`, `projects` pass |
| `core/blocks/tailer.js` | `xenonet-blocks/src/tailer.rs`, with the restart fix | `blocks-tailer`, `live` pass |
| `core/blocks/gatherer.js` | `xenonet-blocks/src/gatherer.rs` | `blocks-gatherer`, `identities`, `claude-usage`, `git-status` pass |
| `core/lines.js` | `xenonet-sources/src/lines.rs` | `lines`, `codex`, `vendor-schemas` pass |
| `core/schema/lenient.js` | `xenonet-schema/src/lenient.rs`, `fields.rs` | `vendor-reader`, `vendor-schemas`, `provider-drift` pass |
| `core/drift.js`, `core/vendor.js` | `xenonet-schema/src/drift.rs`, `vendor.rs` | `drift`, `vendor-reader`, `server` pass |
| `core/blocks/clock.js`, `core/blocks/index.js` | `xenonet-blocks/src/clock.rs`, `lib.rs` (the twin keeps `Date.now()` and the timers, which tests mock) | `xenolith`, and every test that loads a block |
| `core/xenolith.js` | `xenonet-sources/src/xenolith.rs` | `xenolith` passes |
| `core/frames.js`, `core/rename.js`, `core/export-labels.js`, `core/needs-you.js` | `xenonet-state/src/frames.rs`, `rename.rs`, `export_labels.rs`, `needs_you.rs` (the page keeps their JS) | `frames`, `rename`, `export-labels`, `needs-you`, `poster`, `page-markup` pass |
| `core/refusals/demo.js`, `query.js`, `store.js` | `xenonet-sources/src/refusals/` (in xenonet-sources, not xenonet-actors: the live tailer reads refusals) | `refusals`, `refusal-browser` pass |
| `core/redact.js`, `core/digest.js`, `core/classify.js` | `xenonet-state/src/redact.rs`, `digest.rs`, `classify.rs` (running Codex and asking the decider stay in the twins) | `redact`, `digest`, `decide`, `poster` pass |
| `core/openrouter-usage.js` | `xenonet-sources/src/openrouter_usage.rs` (the key and the request stay in the twin) | `costs` passes |
| `core/store.js`, `core/workspace.js` | `xenonet-state/src/store.rs`, `workspace.rs` (the config and the scan's time limit stay in the twins) | `store`, `workspace` pass |
| `core/graph.js` | `xenonet-state/src/graph.rs`, with projects.rs's relations | `graph`, `golden`, `state`, `hosts-hub`, `numbat`, `root` pass |
| `core/state.js` | `xenonet-state/src/state.rs` (the class, its timers and its maps stay in the twin; Rust decides and remembers) | `state`, `golden`, `root`, `schema-output`, `hosts-merge`, `actors`, `server` pass |
| `core/live.js` | `xenonet-sources/src/live/telemetry.rs`, with `live/mod.rs`'s helpers | `live`, `stall`, `signals`, `costs`, `golden`, `state`, `server` pass |
| `core/claude-usage.js` | `xenonet-sources/src/claude_usage.rs` (the keychain, files, fetch and timers stay in the twin) | `claude-usage`, `live` pass |
| `core/refusals/detect.js` | `xenonet-sources/src/refusals/detect.rs` | `refusals`, `refusal-browser`, `live` pass |
| `core/native/copilot-cli.js`, `gemini-cli.js`, `cursor.js` | `xenonet-sources/src/native/{copilot_cli,gemini_cli,cursor}.rs`, with Node's path rules in `xenonet-base`'s `nodepath.rs` (vendor.js's check called back) | `copilot-cli`, `gemini-cli`, `cursor` pass |
| `core/native/opencode.js` | `xenonet-sources/src/native/opencode.rs`, on rusqlite with SQLite bundled (a test's `open` handle runs its statements) | `opencode` passes |
| `core/numbat.js` | `xenonet-sources/src/numbat.rs` | `numbat`, `gemini-cli` pass |
| `core/reported-runs.js` | `xenonet-sources/src/reported_runs.rs` | `space-presence`, `actor-spend`, and `state`, `golden` through core/state.js pass |
| `core/git.js` | `xenonet-sources/src/git.rs` (running git stays in the twin) | `git-status` passes |
| `core/projects.js` | `xenonet-state/src/projects.rs` (mapLimit and limiter stay in the twin) | `projects`, `workspace`, `graph` pass |
| `core/codex.js` | `xenonet-sources/src/codex.rs`, the state database on rusqlite (the app-server's process and requests stay in the twin) | `codex`, `codex-client` pass |
| `core/claude.js` | `xenonet-sources/src/claude.rs` (the Agent SDK's calls stay in the twin) | `claude` passes |
| `core/identities.js` | `xenonet-state/src/identities.rs` (git config's run and the sign-in files' reads stay in the twin) | `identities`, `identities-page` pass |
| `core/schema/check.js` | `xenonet-schema/src/check.rs`, with JS's regular expressions in `jsregex.rs`; zod writes the JSON Schema in the twin | `schema-registry`, `schemas`, `actor-schemas`, `schema-output`, and every test that checks |
| `core/schema/registry.js`, `files.js`, `index.js` | `xenonet-schema/src/registry.rs`, `files.rs` (zod's .meta() and z.toJSONSchema stay in the twin) | `schema-registry`, `schemas`, `vendor-schemas` pass |
| `core/schema/common.js`, the definitions, `vendor/` | Their JSON Schema, which `check.rs` checks against; the zod definitions stay JS, since schemas/ is written from them | every test that loads a schema |
| `core/stats.js` | `xenonet-state/src/stats.rs` (the twin reads the lines through the lines twin) | `stats`, `codex`, `golden` pass |
| `core/search.js` | `xenonet-state/src/search.rs`, its own connection to the same `search.sqlite` | `features`, `lines`, `golden`, `state` pass |
| `core/wiki.js` | `xenonet-sources/src/wiki.rs` | `features`, `xenolith`, `projects` pass |
| `core/blocks/sqlite.js` | `xenonet-blocks/src/sqlite.rs`, on rusqlite with SQLite bundled | No twin (its migrations take a `node:sqlite` handle): Rust tests of the same cases, and the Rust opens a JS-made `engine.sqlite` (version 7, WAL, 18 tables) and `search.sqlite` (an FTS5 match on what node:sqlite indexed) |
| `core/blocks/store.js` | `xenonet-blocks/src/store.rs` | `blocks-store`, `statefile`, `store`, `hosts-config` pass |
| `core/migrate.js`, `core/statefile.js` | `xenonet-state/src/migrate.rs`, `statefile.rs` | `statefile`, `store`, `state` pass |
| `core/dirs.js`, `core/log.js` | `xenonet-base/src/dirs.rs`, `log.rs` | `platform`, `log`, `cli`, `doctor` pass |
| `core/schema/ids.js` | `xenonet-base/src/ids.rs` | `schema-ids`, `hosts-ids`, `schemas` pass |
| `core/client.js` | `xenonet-base/src/client.rs`, `http.rs` (HTTP/1.1 to 127.0.0.1, which the hook uses too) | `client`, `cli`, and the MCP servers' tests through it |
| `core/units.js` | `xenonet-base/src/units.rs`, rounding as `toFixed`, `toLocaleString` and `Math.round` do | `stats`, `token-files`, `state`, `server` pass |
| `core/actors/tokens.js` | `xenonet-actors/src/tokens.rs` | `token-files`, `actor-registration`, `actor-routes`, `actor-gateway` pass |
| `core/private-file.js`, `core/actors/token-file.js` (but the doctor's look in repos) | `xenonet-base/src/private_file.rs`, `xenonet-actors/src/token_file.rs` | `private-file`, `token-files`, `cli-actor`, `mcp-actor`, `hosts-config` pass, the Windows ACL cases included |
| `core/procs.js` | `xenonet-engine/src/procs.rs`, its lookups a machine the twin answers with the commands' output | `procs` and the 20 other files that reach it pass |
| `core/secret-store.js` | `xenonet-actors/src/secret_store.rs` (the store's program still runs in the twin) | `assistant-tunnels`, `actor-wake`, `decide`, `costs`, `integrations`, `spotify` pass |
| `core/actors/products.js`, `detail.js`, `names.js`, `key.js`, `limits.js`, `handoff.js`, `adapters.js`, `spend.js` | `xenonet-engine/src/products.rs`; `xenonet-actors/src/detail.rs`, `names.rs` (on unicode-normalization), `key.rs`, `limits.rs`, `handoff.rs`, `adapters.rs`, `spend.rs` | `actor-names`, `actor-limits`, `actor-spend`, `actors`, `actor-gateway`, `actor-routes`, `gateway-tunnel`, `assistant-connections`, `server` pass |
| `core/engine/store.js` | `xenonet-engine/src/store.rs`, on `sql.rs` (node:sqlite's binding and rows, over rusqlite) | `engine-store` and the 36 other files that reach it pass |
| `core/actors/index.js`, `you.js`, `demo.js`, `attribution.js` | `xenonet-actors/src/registry.rs`, `you.rs`, `demo.rs`, `attribution.rs` (mapOf stays JS, over the state) | `actors`, `actor-needs-you`, `observer`, `observers` and the other files that reach them pass |
| `core/actors/registration.js`, `observer.js` | `xenonet-actors/src/registration.rs`, `observer.rs`, calling the actors through the objects given (`xenonet-engine/src/peer.rs`) | `actor-registration`, `actor-routes`, `actor-gateway`, `observer`, `observers` and the observers' own tests pass |
| `core/actors/app-bundle.js`, `muse.js`, `grok-bot.js`, `hermes-files.js` | `xenonet-actors/src/app_bundle.rs`, `muse.rs`, `grok_bot.rs`, `hermes_files.rs` (files, plutil and Hermes's state.db read through the twin) | `muse-observer`, `grok-bot-observer`, `hermes-files`, `hermes-observer`, `observers` pass |
| `core/actors/hermes.js`, `observers.js` | `xenonet-actors/src/hermes.rs`, `observers.rs` (the observers themselves are built in the twin) | `hermes-observer`, `observers`, `server`, `doctor` pass |
| `core/actors/reports.js` | `xenonet-actors/src/reports.rs` (the actor's processes and their lineage are read through the twin) | `actors`, `actor-routes`, `engine-actors`, `delivery-scopes`, `observers` pass |
| `core/actors/account-links.js`, `remote-presence.js` | `xenonet-actors/src/account_links.rs` (over the engine store), `remote_presence.rs` | `assistant-connections`, `assistant-service` pass |
| `core/actors/connections.js` | `xenonet-actors/src/connections.rs` (over the engine store; the credentials' files and the adapters through the twin) | `assistant-connections`, `assistant-service`, `assistant-endpoint`, `assistant-tunnels`, `assistant-routes`, `server` pass |
| `core/actors/wake.js` | `xenonet-actors/src/wake.rs` (timers, sinks, the credential store, DNS and fetch through the twin) | `actor-wake`, `server` pass |
| `core/engine/verify.js` | `xenonet-engine/src/verify.rs` (procs and the clock through the twin, the pins in the engine store) | `verify`, `delivery-opencode`, `actor-routes` pass |
| `core/engine/attest.js` | `xenonet-engine/src/attest.rs` | `engine`, `engine-actors`, `actor-gateway`, `delivery-opencode` pass |
| `core/engine/connect.js` | `xenonet-engine/src/connect.rs` (the files and the CLIs through the twin) | `engine-connect`, `engine`, `windows-command`, `cli`, `engine-routes` pass |
| `core/engine/engine.js` | `xenonet-engine/src/engine.rs` (the map's state through narrow readers, events, waits and Repos's git in the twin) | `engine`, `engine-actors`, `delivery-wake`, `delivery-scopes`, `actor-wake`, `actor-gateway`, `server` pass |
| `core/delivery/selector.js`, `usage.js`, `policy.js`, `reach.js` | `xenonet-engine/src/delivery/` (the policy's clock, limits and files through the twin) | `delivery-scopes`, `delivery-wake`, `delivery-router`, `delivery-routes`, `delivery-opencode`, `engine` pass |
| `core/delivery/channel.js`, `tmux.js`, `resume.js`, `runs.js`, `setup.js`, `opencode.js` | `xenonet-engine/src/delivery/` (polls, processes, commands, files and HTTP through the twins; a run's process, log and records in the JS runner, given Rust's decisions) | `delivery-router`, `delivery-wake`, `delivery-runs`, `delivery-opencode`, `doctor` pass |
| `core/delivery/router.js` | `xenonet-engine/src/delivery/router.rs` (each record's JS object and the timers in the twin) | `delivery-router`, `delivery-wake`, `delivery-opencode`, `delivery-scopes`, `server` pass |
| `core/config.js`, `core/instances.js`, `core/version.js`, `core/environments.js`, `core/demo-root.js` | `xenonet-app/src/config.rs`, `instances.rs`, `version.rs`, `environments.rs`, `demo_root.rs`, with Node's `path` (both flavors) in `nodepath.rs` | `environments`, `demo-root`, `control`, `golden`, `state`, `store`, `workspace` pass |
| `core/command.js`, `core/open.js`, `core/decide.js` | `xenonet-app/src/command.rs`, `open.rs`, `decide.rs` (the credential store and OpenRouter are awaited by the twin) | `cli`, `doctor`, `windows-command`, `open`, `numbat`, `decide`, `costs`, `integrations` pass |
| `core/mcp-info.js`, `core/mcp-discovery.js`, `core/diagnostics.js` | `xenonet-app/src/mcp_info.rs` (a state machine the twin drives), `mcp_discovery.rs` (the MCP client stays the SDK's for now), `diagnostics.rs` | `mcp-info`, `log` pass |
| `core/script.js`, `core/spotify.js` | `xenonet-app/src/script.rs`, `spotify.rs`: each step, and each Spotify call, a state machine the twin drives | `script`, `spotify` pass |
| `core/control.js` | `xenonet-app/src/control.rs`, on `task.rs`: async Rust whose awaits the twin answers (fetch, the system's processes, spawning) | `control-client`, `control` pass |
| `core/system-stats.js`, `core/themes.js`, `core/themefiles.js` | `xenonet-app/src/system_stats.rs`, `themes.rs` (the page keeps `core/themes.js`; `themes.json` is made from it, and a test checks the two agree), `themefiles.rs` | `system-stats`, `themes`, `theme-gallery`, `theme-menu`, `actors-layer`, `schema-output` pass |
| `core/demo.js` | `xenonet-app/src/demo.rs`; randomness, ids and the clock asked of JS, so the seeded world is the same | `golden`, `actors`, `actor-spend`, `state`, `root`, `hosts-merge` pass |
| `core/components/` (protocol, console-policy, rpc, runtime, packages, install, updates, console) | `xenonet-components`: packages checked against their release's hash and size, then unpacked by its own tar reader behind the same filter; service updates and recovery as tasks | `components`, `component-packages`, `component-activation`, `component-install`, `component-download`, `component-auth`, `component-console`, `launcher`, `assistant-endpoint`, `control-client` pass |
| `core/hosts/` (frames, aliases, cache, doctor, declared, config, link, merge, hub, relay) | `xenonet-hosts`: a link's state as effects the twin performs, every id from a host qualified and every value copied as its schema declares, the relay's scrubbing, shaping and allowlist; zod's checks and graph.js's activityOf through callbacks | `hosts-cli`, `hosts-config`, `hosts-doctor`, `hosts-frames`, `hosts-hub`, `hosts-ids`, `hosts-link`, `hosts-merge`, `hosts-relay`, `hosts-routes`, `hosts-server` pass |
| `core/assistant/` (protocol, audit, cursors, credentials, fixtures, projection, source, service, client, tunnel-client, tunnels) | `xenonet-assistant`: grant credentials written private in the JS's order, signed page tokens (HMAC-SHA256 on sha2), what a grant sees and its labels cleaned, the six reads as tasks, the adapter's retries and standing, tunnels' runs and the client's own environment; zod's checks, the grants' standing and rate, and the processes stay in the twins | `assistant-connections`, `assistant-endpoint`, `assistant-routes`, `assistant-service`, `assistant-source`, `assistant-tunnels`, `schemas` pass |
| `core/integrations/` (common, routing, pi, reviews, openshell, index) | `xenonet-integrations`: credentials redacted, tickets and records, the routing answer's check, Pi metadata, the frozen text snapshot (O_NOFOLLOW reads, stamps before and after), what a sandbox returns validated as untrusted, the patch, and OpenShell's argv and flows as tasks; JevRouter, git and the in-sandbox collector (JS run inside the sandbox) stay in the twins | `integrations`, `integrations-routes`, `sandbox-reviews` pass |
| `core/doctor/` (index, service, checkup, context) | `xenonet-doctor`: each check's verdict and words as a task asking the twin's ctx (files, processes, XenoNet's state) for what it reads, the hosts' checks, the summary, new problems and the last run's file; machine(), the runner's time limits and the schedule stay in the twins | `doctor`, `observers`, `token-files`, `hosts-doctor`, `schema-output` pass |
| `xenonet mcp` (`mcp/xenonet.js`), for sessions, actors and channel sessions | `xenonet-mcp`, on rmcp | Contract tests against the JS server (initialize, `tools/list`, the probe, in each kind of session); `mcp-actor`, `engine` pass against the binary |
| `xenonet mcp assistant` (`mcp/assistant.js`, with `core/assistant/client.js`) | `xenonet-mcp` | Contract tests against the JS server, for a grant and against its `--fixture`; `assistant-endpoint`, `e2e/assistant` pass against the binary |
| `xenonet mcp console` (`mcp/console.js`, over `core/control.js`) | `xenonet-mcp/src/console.rs`, through `xenonet-cli/src/cores/control.rs` | Contract tests against the JS server (the handshake, `tools/list` key for key, its refusals, in both revisions); `control` passes against the binary |
| `xenonet hook` and every other command (`cli/`) | `xenonet-cli`, the `xenonet` binary, its cores driven in `src/cores` | `cli`, `cli-actor`, `cli-gateway`, `cli-status-agents`, `hosts-cli`, `launcher` pass against the binary |
| `xenonet serve` (`server/index.js`, `server/routes/`, `component-service.js`, `static.js`, `upstream.js`) | `xenonet-service`, its cores driven by `xenonet-wire-map`, `-engine` and `-app` | The JS route tests' cases as Rust tests of the routes; `server`, `hosts-server`, `engine`, `control` pass against the binary; `scripts/rust-diff.mjs` asks it and the JS service the same reads side by side |
| The actor gateway, its switch and tunnels (`server/actor-gateway.js`, `gateway-control.js`, `gateway-tunnel.js`, `gateway-reach.js`) | `xenonet-service/src/gateway`, the public address check over its own DNS (`reach.rs`) | 69 of the JS gateway's answers, byte for byte; `gateway-tunnel`'s cases |
| The terminal drawer's shells (`server/terminals.js`) | `xenonet-service/src/terminals.rs`, on portable-pty | Its own Rust tests, a real shell's among them |
| The assistant's endpoint (`server/assistant-endpoint.js`) | `xenonet-service` | Its hello, read and sweep, as a Rust test |

With every twin in place, all 131 test files that reach one pass (of 170; the other 39 load no module that has a twin). The
twins load in the Node processes a test starts too (a server, the
command, an MCP server), so the route and command tests run on them. The two
fixes on `fix/scanner-racy-mtime` are in the Rust (the scanner reads a
folder again when it read it just after it changed, and the tailer
catches up the files it followed as live after a restart), and that
branch's new tests pass against them too.

A twin is the contract's check, not yet something to ship: reading a
transcript through the lines twin takes about 8 times as long as the JS,
since each record crosses between JS and Rust several times while the
vendor checks are still JS. The readers gain from Rust once they're Rust
from the file to the answer.

The readers and the schemas (step 2) are ported: every module of `core/` that reads a source, the map and its state, and `core/schema/`. Through the twins they're slower than the JS (a map of 3,000 sessions takes about 26 ms against 5 ms, a session's stats about 6 times as long), since every value crosses between JS and Rust; they gain once they're Rust from the file to the answer. The engine, delivery, the actors, the assistant, hosts and the binary followed (steps 3 and 4), so every core is in Rust; in the binary, crates of drivers run them natively (see "Decided along the way").

`npm run test:bin` builds the binary and runs every JS test that starts
the service, the command or an MCP server against it
(`XENONET_SERVICE_BIN`, `XENONET_CLI_BIN`, through
`test/harness/xenonet-bin.js`), and `crates/xenonet-cli/bin-tests.json`
keeps the counts. Desktop runs `xenonet serve` and nothing else
(`electron/service-runtime.js`): the installed Service's `bin/xenonet`,
or in XenoNet Dev the checkout's newer build of `target/debug` and
`target/release`. With no binary it stops and says how to get one. The
Service and Tools packages are the binary, as `bin/xenonet`, with no Node
runtime or JS beside it (`scripts/component-build.mjs`); the binary has
the node launcher script built in, for entries an earlier XenoNet wrote.
A package's startup check runs `bin/xenonet --version --json`, which says
the version in the package's `package.json`. XenoNet
Dev's page still hot-reloads, through Vite in Desktop's own frontend
server (`electron/desktop-server.js`), and it restarts its service when
the binary is rebuilt.
