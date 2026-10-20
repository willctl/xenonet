# Threat model

XenoNet maps the Claude Code and Codex sessions on a computer and lets them message each other. It's a desktop app (Electron, `electron/`, and a page, `src/`) over a background service: the `xenonet` binary, built from `crates/`, which is also the `xenonet` command and XenoNet's MCP servers. `SECURITY.md` says what's in scope and `docs/guide/privacy.md` what XenoNet promises; this file is for the scan.

## What this project does and where untrusted input enters

The service runs as the person's own user and listens on `127.0.0.1` only. Treat these as untrusted:

- **Web pages in the person's browser.** Any page can send requests to `127.0.0.1:47840`. The service answers only its own `Host` (`crates/xenonet-service/src/serve.rs`, 421 otherwise, against DNS rebinding) and refuses a cross-origin request (403). The console page runs under a Content-Security-Policy, whether the service serves it to a browser (`crates/xenonet-service/src/static_files.rs`) or Desktop serves it to its window (`server/static.js`, through `electron/desktop-server.js`), so text from transcripts, actors or other hosts must never become script.
- **Transcripts and other apps' files.** The readers parse files other programs write: Claude Code's and Codex's JSONL transcripts, Codex's state database, OpenCode's SQLite database, Cursor's, Copilot CLI's and Gemini CLI's files, the shared wiki (`crates/xenonet-sources`), and a few other apps' `Info.plist` keys and metadata (`crates/xenonet-actors`). A transcript holds whatever a model or a tool printed, so assume an attacker wrote it.
- **Repositories.** XenoNet runs `git status` on every repo under the projects folder. A cloned repo's config must never make it run a program: the repo's filters are emptied, its hooks pointed at nothing and its fsmonitor turned off (`crates/xenonet-sources/src/git.rs`).
- **Other programs on the same computer.** Any local program can call the HTTP API and the MCP servers and claim a session's name. On macOS the service looks up the process on the other end of each connection and its parents (`crates/xenonet-engine/src/attest.rs`), so one session can't act as another, and requests from under a session can't connect Claude Code and Codex or change standing settings. Windows and Linux don't check this yet, as `docs/guide/privacy.md` says.
- **Actors.** Programs the person allowed (`crates/xenonet-actors`) authenticate with tokens kept only as SHA-256 hashes, and reach only the sessions and actors their scopes name (default deny).
- **The internet, through the actor gateway.** Off by default. When on, it listens on the computer and the person's tunnel (Tailscale, cloudflared or ngrok) puts it on the internet. It answers only its own address, only at `/mcp`, only to gateway tokens, with rate limits (`crates/xenonet-service/src/gateway`). Assume the attacker holds no token, or holds one actor's token and wants more than that actor may do.
- **Other XenoNets, through relays.** With other machines turned on, XenoNet reads them over SSH (`xenonet relay`, `crates/xenonet-hosts`). A host's frames are untrusted: they're checked and capped, and a host's ids must never pass for this machine's or another host's.
- **Remote actors, through the assistant tunnel.** A paired remote actor reads statuses and counts of the projects it was given, through `crates/xenonet-service/src/assistant_endpoint.rs` and `crates/xenonet-assistant`, and must never read transcripts, prompts, paths or credentials.
- **Updates.** Component packages (the Service and Tools archives) are checked against their manifest's SHA-256 before anything is written, and every archive entry is checked before it's extracted (`crates/xenonet-components`).

## Components that matter most / least

Most: anything reachable from a web page or the gateway (`crates/xenonet-service/src/serve.rs`, `routes/`, `gateway/`); the checks that tell callers apart (`attest.rs`, the window key in `crates/xenonet-actors/src/key.rs`, actor scopes and tokens); the transcript and file readers; the git invocation; the component installer; what XenoNet writes into Claude Code's and Codex's settings (`crates/xenonet-engine/src/connect.rs`); and the hook command (`xenonet hook`), which those programs run.

Also in scope: Desktop (`electron/`): its frontend server, which adds the page's secret to the window's own API requests (`electron/desktop-server.js`); what it opens for the service (only apps' links and absolute paths); and the IPC it offers the page. The terminal runs in the service and is reached only over its local socket, by Desktop's connection.

Less: `core/`, `server/`, `cli/` and `mcp/` hold the JavaScript the Rust was ported from. It's the contract the Rust is tested against, and Desktop imports a few `core/` modules, but the service, the command and the MCP servers that run are the binary. A finding there matters where Desktop runs that module, or where the Rust behaves the same way.

## How to exercise it

The image has everything built:

- `target/debug/xenonet serve --demo` runs the service on made-up data (a demo world in a temporary folder) on `127.0.0.1:47841`. Without `--demo` it reads `$HOME`'s real transcript folders, which in the image are empty: write transcripts under `$HOME/.claude/projects/<project>/<session>.jsonl` to feed it. `--dev` adds the dev instance's routes (`/api/dev/simulate`, `/api/dev/capture`).
- `target/debug/xenonet mcp` is the MCP server on stdio; `target/debug/xenonet hook claude` reads a hook's JSON on stdin and posts it to the service; `target/debug/xenonet relay` is what another XenoNet reads over SSH.
- `cargo test --workspace` runs the Rust tests. `npm test` runs the JS tests against the JS; `npm run test:rust` runs them with each ported module's Rust twin loaded; `npm run test:bin` runs the ones that start a service, a command or an MCP server against the binary (`test/harness/xenonet-bin.js`).
- `test/server.test.js`, `test/actor-gateway.test.js`, `test/engine-routes.test.js` and `test/hosts-server.test.js` show how the HTTP API, the gateway and relays are called.

## How you rate severity

- **Critical:** running code or reading files as the person, from a web page, the gateway, a remote actor, another host, a repo or a transcript; an update that installs code that doesn't match its manifest.
- **High:** a web page, the gateway, another host or a remote actor reading transcripts, prompts, messages, tokens or files; getting past the `Host` or origin checks; on macOS, one session or actor acting as another; an actor reaching a session or actor its scopes don't name; changing Claude Code's or Codex's settings, or XenoNet's standing settings, from under a session; writing outside XenoNet's own folders and the settings entries it manages.
- **Medium:** a panic, hang or unbounded memory in the service from a transcript, a relay frame, a gateway request or an MCP message (it runs for days and serves every session); showing more than `docs/guide/privacy.md` says a reader may see, such as a home folder where `~` should show.
- **Low:** what needs a program already running as the same user with no session to impersonate, since that program can read the same files directly.

## Anything to leave alone

- Claude Code, Codex, Electron, Node and the apps XenoNet observes: their bugs belong to their projects.
- Anything that needs administrator rights or physical access.
- That Windows and Linux don't check which process a request comes from: it's documented and planned.
- `xenolith/` is a separate project with its own `SECURITY.md`. `scripts/`, `test/` and the demo world's made-up data (`core/demo.js` and its Rust port) are development tools.
