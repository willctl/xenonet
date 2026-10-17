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
  xenonet-sources          lines.js, claude.js, codex.js, git.js, wiki.js, live.js, numbat.js, demo.js, native/
  xenonet-state            graph.js, state.js, search.js, digest.js, stats.js, identities.js, projects.js,
                           workspace.js, classify.js, store.js, statefile.js, migrate.js
  xenonet-engine           engine/, delivery/, procs.js
  xenonet-actors           actors/, assistant/, secret-store.js, integrations/, refusals/
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
`core/` do today. `server`, `mcp` and `cli` are thin layers over the rest
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
| `zstd`, `flate2` | Codex's archived rollouts (`.jsonl.zst`) and the refusals' gzip context. |
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
- **Events.** A block's EventEmitter stays in its twin. Rust returns each
  state change, and the twin emits it.

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
- The actor gateway on its own port (the API's plus 10) while it's on,
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

Every current `rmcp` release needs Rust 1.88 or later (through let-chains
and `darling` 0.23). The workspace builds on Rust 1.85, and
`crates/xenonet-mcp` is a workspace of its own for now, built with a newer
toolchain (`rustup run 1.90.0 cargo test --manifest-path
crates/xenonet-mcp/Cargo.toml`). Once a `rust-toolchain.toml` pins a newer
toolchain for the checkout (rustup installs it there without changing
anyone's default), the MCP crate joins the workspace.

## Hooks

`xenonet hook <app>` stays what it is: it reads the hook's JSON on stdin,
posts it to `/api/hook/<app>` with a 4-second timeout, prints the answer
on a 200, and always exits 0. Claude Code's HTTP hooks don't run the
command at all and don't change.

What changes is the command XenoNet writes into Claude Code's and Codex's
settings and MCP lists: the binary instead of `'<node>' '<entry>'`, and
no `ELECTRON_RUN_AS_NODE`. `connect` recognizes both forms as its own, so
connecting again replaces the old entries, and `disconnect` takes out
either. Until then the preview shows the old entries as different, and
the doctor says to connect again.

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
5. **The switch.** Desktop starts the binary as the standalone service.
   The JS core and the addon go, and the remaining tests move to
   `cargo test`.

Each step leaves `npm test` passing on JS, and adds to `parity.json`.

## Not decided yet

- When to pin a newer Rust toolchain: with the MCP crate, or before.
- `hyper` with XenoNet's own router (proposed above), or a framework.
- Whether the `xenonet-console` MCP server, which drives Desktop's
  control channel, moves to Rust or stays a JS tool in the Tools package.
- Where terminals live: in the Rust service (with a pty crate), or in
  Electron, as Desktop's own already do.
- Signing on Windows.
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
| `core/blocks/sqlite.js` | `xenonet-blocks/src/sqlite.rs`, on rusqlite with SQLite bundled | No twin (its migrations take a `node:sqlite` handle): Rust tests of the same cases, and the Rust opens a JS-made `engine.sqlite` (version 7, WAL, 18 tables) and `search.sqlite` (an FTS5 match on what node:sqlite indexed) |
| `core/blocks/store.js` | `xenonet-blocks/src/store.rs` | `blocks-store`, `statefile`, `store`, `hosts-config` pass |
| `core/migrate.js`, `core/statefile.js` | `xenonet-state/src/migrate.rs`, `statefile.rs` | `statefile`, `store`, `state` pass |
| `core/dirs.js`, `core/log.js` | `xenonet-base/src/dirs.rs`, `log.rs` | `platform`, `log`, `cli`, `doctor` pass |
| `core/schema/ids.js` | `xenonet-base/src/ids.rs` | `schema-ids`, `hosts-ids`, `schemas` pass |
| `core/client.js` | `xenonet-base/src/client.rs`, `http.rs` (HTTP/1.1 to 127.0.0.1, which the hook uses too) | `client`, `cli`, and the MCP servers' tests through it |
| `core/units.js` | `xenonet-base/src/units.rs`, rounding as `toFixed`, `toLocaleString` and `Math.round` do | `stats`, `token-files`, `state`, `server` pass |
| `core/actors/tokens.js` | `xenonet-actors/src/tokens.rs` | `token-files`, `actor-registration`, `actor-routes`, `actor-gateway` pass |
| `core/private-file.js`, `core/actors/token-file.js` (but the doctor's look in repos) | `xenonet-base/src/private_file.rs`, `xenonet-actors/src/token_file.rs` | `private-file`, `token-files`, `cli-actor`, `mcp-actor`, `hosts-config` pass, the Windows ACL cases included |
| `xenonet mcp` for sessions (`mcp/xenonet.js`), a prototype | `xenonet-mcp`, on rmcp, Rust 1.88+ | Its own stdio tests; the same answers as the JS servers to the probe and the handshake |
| `xenonet hook` (`cli/commands/hook.js`) | `xenonet-cli`, the `xenonet` binary | `launcher`'s hook cases, as Rust tests of the binary |

With every twin in place, all 111 test files that reach one pass. The
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

Not ported yet: everything else, in the order above.
