# Development

## Commands

```bash
npm run app       # the desktop app (Electron) from source, as XenoNet Dev
npm run dev       # XenoNet Dev's service alone, with the UI at http://127.0.0.1:47842
npm run demo      # the same, with made-up data, at http://127.0.0.1:47841
npm run rust:bin  # the xenonet binary: the service, the command and the MCP servers
npm run dev:install  # macOS: XenoNet Dev.app in ~/Applications, running this checkout
npm test          # unit tests
npm run test:e2e  # end-to-end tests, on a hidden XenoNet Dev with made-up data
node cli/xenonet.js console launch shot:overview  # drive XenoNet Dev from a terminal (see below)
npm run lab -- status  # macOS: the VM lab, Linux VMs to try other machines in (see below)
npm run dist      # the installers for this OS, in release/ (dist:win, dist:mac)
npm run test:bin  # the tests that start the service, the command or an MCP server, against the binary
npm run rust:build  # the Rust core's Node addon, which the JS tests load in place of core/ (test:rust)
npm run test:rust   # the unit tests, with the Rust core in place of the JS
npm run lint:rust   # formatting, clippy, docs and the dependency checks
cargo test        # the Rust core's own tests
npm run coverage:rust  # how much of the Rust core the tests run, crate by crate (see Coverage)
```

## XenoNet Dev

The desktop app run from source is XenoNet Dev. It has its own settings, port, layout and search index, so it runs beside an installed XenoNet without writing over its files. Its port is 47842. The first run copies the installed XenoNet's layout. `npm run dev`, the service alone, uses XenoNet Dev's port and folders too.

XenoNet's service is the `xenonet` binary, built from `crates/` with the Rust that `rust-toolchain.toml` pins (rustup installs it on the first build). XenoNet Dev runs the checkout's newer build of `target/debug/xenonet` and `target/release/xenonet`. `npm run app`, `npm run dev` and `npm run test:e2e` build it first; started any other way, XenoNet Dev says to run `npm run rust:bin` when there's none. The service runs on its own, as an installed one does, and keeps running when XenoNet Dev quits.

Connecting your agents from XenoNet Dev adds its own entries beside the installed XenoNet's, with its MCP server named `xenonet-dev`, and leaves the installed XenoNet's entries alone. Disconnecting takes away only its own. Whichever XenoNet connects next clears entries that point at a checkout that's gone. Terminals XenoNet opens don't pass on its own `XENONET_*` settings, so `npm run app` started in one runs as XenoNet Dev, not on the installed XenoNet's port.

XenoNet Dev picks up edits as you save them:

- The UI hot-reloads. `npm run dev` rebuilds it into `dist/` instead: reload the page to see a change.
- A change under `crates/` takes a build: `npm run rust:bin`. XenoNet Dev sees the binary change, stops its service and relaunches onto the new one.
- A change under `electron/`, or to the `core/` and `server/` modules Desktop runs itself, relaunches XenoNet Dev.app. If you started XenoNet Dev with `npm run app`, it prints a reminder to restart instead.

Vite keeps its prebundled dependencies in each checkout's `.xenonet/vite`, which git ignores. So worktrees that link `node_modules` don't share them.

### Linux and WSL2 checks

On a Windows host where application control blocks unapproved native scripts or executables, run XenoNet development tools, builds and tests inside WSL2 or Docker. Keep the source checkout in your normal projects folder. Install Linux dependencies from the Linux shell rather than reusing Windows `node_modules`.

Run Electron checks as a regular Linux user. A headless display needs Xvfb and a WebGL renderer. Use `XENONET_E2E_SHOW=inactive XENONET_SOFTWARE_RENDERING=1 xvfb-run -a -s "-screen 0 1920x1080x24" npm run test:e2e`. The screen holds Desktop's 1600 by 960 window, which Xvfb's default screen would narrow. Chromium's software renderer draws windows inside Xvfb's isolated display. Keeping them visible there prevents hidden-window frame throttling and stale captures. The renderer sandbox stays enabled, and installed app settings stay as they were. Each test port gets its own Electron profile.

CI runs the whole suite this way on Ubuntu 24.04, in the `e2e-linux` job of `.github/workflows/ci.yml`, in about 50 minutes on a hosted runner: drawn in software, the map makes a frame every second or two. With `XENONET_SOFTWARE_RENDERING=1` the tests' time limits stretch to match (`test/harness/software.js`). Ubuntu 23.10 and later keep unprivileged programs from making the user namespaces Chromium's sandbox needs, so Electron stops at launch with a message about `chrome-sandbox`. The job loads an AppArmor profile that allows them for the checkout's Electron alone, and its step shows how to do the same on your own machine.

For machines with many cores, `npm test -- --test-concurrency=4` keeps the unit suite's process count bounded. Review/export verification with a real OpenShell gateway is described in [Agent workflows](integrations.md#storage-and-verification). Its fixture makes no model or provider calls.

`npm run dev:install` puts XenoNet Dev.app in `~/Applications` and opens it. Then you can test the Dock, the menu bar and notifications without building an installer. XenoNet Dev.app is the Electron that `npm install` fetched, renamed and given XenoNet's icon, with a pointer to this checkout. So code changes never need a reinstall. Run `npm run dev:install` again after Electron updates, or if you move the checkout. To remove XenoNet Dev.app, drag it to the Trash.

## The page's Content-Security-Policy

The page runs under a Content-Security-Policy, set in `server/static.js`, and in `vite.config.js` for XenoNet Dev. Scripts load only from XenoNet's own server, with nothing inline and no `eval`, and the page connects only to that server. An inline script, an `eval` or a script from elsewhere won't run, so don't add one.

## The VM lab

`npm run lab` builds a lab of Linux VMs on a Mac, with Lima 2.1 or later, to try [other machines](other-machines.md) against. Each VM runs Claude Code, the Codex CLI and a headless XenoNet, with made-up repos. The XenoNet on the Mac reaches each VM over SSH with `xenonet relay`.

`npm run lab -- check` checks that each VM's XenoNets answer, that the VM is walled off from the Mac, and that only made-up names are in it.

[scripts/lab/README.md](../../scripts/lab/README.md) says how to build and use the lab. Signing in inside the VMs, and adding them as hosts to a console, are yours to do.

## Developing on another computer

Claude Code and Codex can run on one computer while you develop XenoNet on another, such as a VM. The sessions' computer, where Claude Code and Codex run, keeps an installed XenoNet with every part. The other runs XenoNet Dev from the checkout. Two forwarded ports join them, both opened over SSH from the sessions' computer, so nothing listens on a network. Each port keeps its number on the other side, since XenoNet answers only to its own address:

```sh
# On the sessions' computer, its XenoNet (47840) reaches the other computer as 47840,
# and that computer's XenoNet Dev (47842) answers here as 47842.
ssh -N -R 127.0.0.1:47840:127.0.0.1:47840 -L 127.0.0.1:47842:127.0.0.1:47842 you@devbox
```

- **The page against real sessions.** On the other computer, `XENONET_UPSTREAM=http://127.0.0.1:47840 npm run dev` serves the page from the checkout and passes its reads to the installed XenoNet (`crates/xenonet-service/src/upstream.rs`). Only reads go through: anything that would change something there is refused, so a page in development can look but never act. Add `--demo` to keep the dev server's own service on made-up data. `XENONET_UPSTREAM` takes only an address on that computer, a forwarded port, and the installed service ignores it.
- **Sessions talking to XenoNet Dev.** On the sessions' computer, `xenonet connect --here --port 47842` adds XenoNet Dev's MCP server and hooks to Claude Code's and Codex's settings beside the installed XenoNet's, through that computer's own launcher. Asking XenoNet Dev itself would write the other computer's settings. While the forward is down, its hooks do nothing. `xenonet disconnect --port 47842` takes them out again.

What acts on the sessions' computer (opening a session in its app, handoffs, terminals in its folders, its credential store) stays with the installed XenoNet there.

## Building

- `npm run dist:dir` builds only the app, such as `release/win-unpacked/XenoNet.exe` or `release/mac-arm64/XenoNet.app`. On Windows, `npm run shortcut` puts a desktop shortcut to it.
- macOS builds are ad-hoc signed, and the package is unsigned. The build stops if a certificate signs either one, because a certificate carries its owner's name. To sign on purpose, with a Developer ID, set `XENONET_SIGN=1`. The build also stops if a packaged file contains your home folder's path.
- The terminal's native module ships one npm package per platform and CPU. The build fetches the one each app needs, so one Mac builds both the Apple silicon and the Intel app.
- The icon is drawn in code, in `electron/icons.js`, at every size from 16 to 256 pixels, with a macOS cut on Apple's icon grid. `npm run icon` regenerates it, along with the installer art for both platforms.

[Install](install.md) says how to install what you build.

## Coverage

`npm run coverage:rust` measures how much of the Rust core the tests run, with cargo-llvm-cov. It runs `cargo test` instrumented, then prints each crate's lines and the share of its lines, functions and regions the tests ran.

- `-- --js` counts the JS tests too: the ones `npm run test:rust` runs on the twins, and the ones `npm run test:bin` runs against the binary. Run `npm run build` first, as for `test:bin`.
- `-- --html` writes the report, line by line, to `target/llvm-cov/html/index.html`.
- `-- --no-run` shows the last run's numbers again, and `-- --json` gives them as JSON.

It needs cargo-llvm-cov and the pinned toolchain's llvm-tools:

```sh
cargo install --locked cargo-llvm-cov
rustup component add llvm-tools --toolchain 1.99.1  # the channel rust-toolchain.toml pins
```

The instrumented builds go to `target/llvm-cov-target` and `target/llvm-cov-js`, apart from the usual ones, so `cargo test`, `npm run lint:rust` and the addon don't build again after a run. Once built, a run takes about a minute, and `--js` about four. While `--js` runs, the twins load an instrumented addon, and the one that was there is put back after. llvm-cov counts the `#[cfg(test)]` modules in a crate's own files among its lines.

On 2026-10-19 the tests ran this share of each crate's lines:

| Crate | `cargo test` | With the JS tests |
| --- | --- | --- |
| xenonet-actors | 40.7% | 93.0% |
| xenonet-app | 69.0% | 90.2% |
| xenonet-assistant | 58.8% | 94.7% |
| xenonet-base | 81.5% | 91.2% |
| xenonet-blocks | 85.8% | 97.2% |
| xenonet-cli | 21.4% | 55.5% |
| xenonet-components | 59.7% | 89.7% |
| xenonet-doctor | 42.8% | 92.9% |
| xenonet-engine | 68.1% | 91.1% |
| xenonet-hosts | 33.6% | 95.2% |
| xenonet-integrations | 53.9% | 93.9% |
| xenonet-mcp | 56.6% | 56.9% |
| xenonet-node | (not run) | 77.2% |
| xenonet-schema | 72.3% | 84.9% |
| xenonet-service | 60.0% | 70.0% |
| xenonet-sources | 79.6% | 91.6% |
| xenonet-state | 82.4% | 93.6% |
| xenonet-wire-app | 54.0% | 66.2% |
| xenonet-wire-engine | 55.3% | 60.7% |
| xenonet-wire-map | 74.4% | 80.4% |
| All | 59.9% | 81.3% |

The JS tests reach most of the core through the twins, and the binary only through `test:bin`'s files, so the binary's own parts (`xenonet-cli`, `xenonet-mcp`, `xenonet-service` and the `xenonet-wire-*` crates) are the least covered.

## Fuzzing

XenoNet reads files, hook payloads, frames and requests that other programs write, so none of it can be trusted. `fuzz/` has fuzz targets for those readers, run with [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz). It's a workspace of its own, apart from `crates/`, because libFuzzer needs a nightly Rust. The pinned toolchain, `cargo deny` and `cargo shear` never see it.

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
fuzz/run.sh lines 1800      # one target for 30 minutes
```

`cargo fuzz list --fuzz-dir fuzz` lists the targets. Each one's file in `fuzz/fuzz_targets/` says what it reads:

| Target | What it reads |
| --- | --- |
| `json` | JSON as XenoNet parses and writes it |
| `lines`, `telemetry` | Claude Code transcripts and Codex rollouts, line by line, into the live map |
| `transcript_files` | Transcript files on disk: the session listing, the lenient normalizers, rollout pages, archived rollouts' Zstandard, stats |
| `schema`, `jsregex` | Records checked against the vendor schemas and XenoNet's own, and the schemas' patterns |
| `native`, `sources`, `wiki` | The other apps' files (Gemini CLI, Copilot CLI, OpenCode, Cursor, numbat), Codex's app-server, git's output, plan usage, the shared context |
| `base` | Strings, times, paths, git remotes and model names |
| `http`, `hooks` | The service's routes, hook payloads and the actor gateway |
| `state`, `hosts`, `misc` | `state.json`, frames from other machines, component packages, tokens and the remote actors' requests |
| `tools` | What other tools print or keep: gh's and ssh's config, git's, ps, lsof, netstat, tmux, Codex's picks |
| `net_frame`, `net_roster` | The network's frames and roster entries, checked against a roster and opened |
| `net_request`, `net_bodies` | The gateway's routes, the answers a host reads from it, and the bodies the engine takes from other machines |
| `net_relay` | The map between machines: a console's asks at a host's relay, the paths it serves, and the relay's answers at the console's hub |
| `a2a_rpc` | A2A's JSON-RPC requests and its event streams |

The network's targets build well-formed frames, entries and requests from a made-up network with fixed keys, then change them, so a run reaches past the signature checks. They fail if a changed frame or request is ever taken.

`fuzz/run.sh` keeps what a target learns in `fuzz/corpus/<target>` and reads the seeds in `fuzz/seeds/<target>` too. A crash is saved in `fuzz/artifacts/<target>`. Run the target on that file alone to see the crash again, and `cargo fuzz tmin --fuzz-dir fuzz <target> <file>` makes it smaller. The corpora and crashes stay out of git.

Fix a crash where it starts, and add a unit test with the smallest input that showed it, in the crate that had the bug. The fuzz crate needs nightly, so it holds no tests of its own. The seeds come from the test fixtures and the demo's made-up world, never from real sessions: `node fuzz/seed.mjs` writes them again, and stops if one would hold this machine's home folder.

## Driving XenoNet from agents, scripts and tests

Installed XenoNet and XenoNet Dev both have a control channel, so an agent or a script can launch Desktop, act on it and see it the way you would. Development copies also offer simulation and evaluation for tests. Use the demo to check a change to XenoNet. Agents can operate the installed console for everyday work. [Components](components.md) explains the production MCP setup.

The control channel operates the console. The engine that connects sessions to each other is a separate MCP server, `xenonet`, in `mcp/xenonet.js`. See [Sessions working together](agents.md).

- Agents can use the control channel through the `xenonet-console` MCP server's tools, including navigation, search, screenshots, terminals, themes and updates. Installed Tools omit `eval` and `simulate`, which are only for development. `actors` includes the programs waiting for an Allow, and how each token stands. `needs_you` reads the Needs you queue and shows, opens or dismisses an item, but never allows or denies one. `feed` is the activity feed. `mcp_info` opens a shared system's metadata, tools and context catalog without executing tools. `environment` changes the surroundings independently of the theme. `look` draws the panels solid, without the glass blur behind them, or as glass again (`state.look`).
- `.mcp.json` registers these tools for Claude Code, which asks you once to approve them. `.codex/config.toml` registers them for Codex, in a trusted project. Start Claude Code or Codex in the repo's root folder.
- From a terminal, `xenonet console <steps>` runs the same actions one after another, for example `xenonet console launch select:atlas-api shot:atlas`. `npm run drive -- <steps>` does the same. `xenonet console --help` lists the steps.
- XenoNet Dev keeps a log of what you do in it, as the same steps. Selecting a node, a palette command, a session's stats, a theme and a film all go in the log. `xenonet script record --instance dev > tour.json` saves it, and `xenonet script run tour.json` plays it back, in any instance, shown or hidden.
- A script is a JSON list of steps, written as strings such as `"select:atlas-api"`, or as objects such as `{ "op": "select", "args": [...] }`. Any driver method is a step. For anything else, a script can be a JS module whose default export gets the instance and a `step` function.
- `npm run test:e2e` drives a hidden XenoNet Dev with the demo's data through the same channel, in about five minutes. Run it before you send a change to the UI. A failing test saves a screenshot of what it saw to `.xenonet/`.
- Starting the service at login is tested without the machine's own launchd or systemd. `XENONET_SUPERVISOR_BIN` names the `launchctl` or `systemctl` the `xenonet` command runs, and `XENONET_SUPERVISOR_DIR` the folder the launch agent or unit goes in. `test/e2e/background.test.js` points both at a made-up supervisor in a temporary folder.
- `npm run test:real` checks, on a real Mac, that Grok Bot and Muse message each other through XenoNet Dev (the dev instance), through the actor gateway, or with `XENONET_REAL_ROUTE=local` from commands their apps run. It needs both apps signed in, you at XenoNet's window to set up the gateway and connect each, and someone to ask Grok Bot and Muse to do what it prints. It only reads XenoNet's state. `XENONET_REAL_PORT`, `XENONET_REAL_WORD` and `XENONET_REAL_MINUTES` change where it looks, the word the messages carry and how long it waits.
- Scripts import `core/control.js`, which all of the above use.

### Waiting and screenshots

Every action waits until the view has settled, then returns the new state, so nothing sleeps for a fixed time. The view has settled once the layout has cooled, the camera has stopped and any gathering has arrived.

A screenshot waits the same way. It also holds every ambient animation still, including the sky, the sonar, the sweep, sparks and CSS animations, so two shots of the same state match. Screenshots work while the window is covered, unfocused or hidden. They land in `.xenonet/`, which git ignores.

The map draws at a pace, not at the display's rate (`src/render-budget.js`): 60 frames a second while something moves, 30 at rest, 20 at rest behind another app, and none while nobody can see the window, unless a recording needs every frame. While the control channel has a request in hand, and for two seconds after, the window counts as driven and draws even while hidden or covered, so the view settles and screenshots show it. `state` says how the map is drawing (`render`), and the driver's `render` counts the frames it draws over a second or more. `npm run bench:gpu` measures what that costs the machine in each window state, and `npm run bench:gpu -- --plain` with plain panels, the palette's setting that draws the panels without their glass blur.

### Instances

Development instances each have their own settings and port, so they run beside installed XenoNet and each other.

| Instance | Data | Port | Window, when an agent starts it |
| --- | --- | --- | --- |
| `dev` | Your sessions | 47842 | Shown without taking focus |
| `demo` | Made-up projects and sessions | 47841 | Shown without taking focus |
| `test` | The demo's, for `npm run test:e2e` | A spare one for each test run, or else 47843 | Hidden, with no sound, notifications or tray icon, and no reloading as you edit |
| `film` | Your sessions, for `xenonet film` | 47844 | Hidden like `test`, so a film renders while you work |
| `installed` | Your sessions | 47840 | The installed Desktop; its service keeps running when the window quits |
| `showcase` | Made-up projects and sessions | 47845 | Frozen for the [public demo recording](showcase.md) |

Agents and `xenonet console` use `demo` unless told otherwise, so nothing private ends up in their screenshots. Opened from the Dock or with `npm run app`, XenoNet Dev is `dev`. `XENONET_INSTANCE` picks another instance. `XENONET_SHOW=inactive` or `XENONET_SHOW=hidden` starts the window the way an agent does.

### Launching and quitting

If a copy of XenoNet Dev is starting or quitting, `launch` waits for it.

A demo or test copy can get stuck, holding the instance's lock without answering. `launch` ends such a copy by itself, since its data is made up, and says so.

`launch:restart`, or `restart` on the tool, quits a running copy first. It also ends a stuck copy of any instance, `dev` included.

`quit` returns once XenoNet Dev has exited. A copy still there 20 seconds after its server went is killed. For the `test` instance, that takes its whole process tree: the app, its GPU and network processes, and the service it started, with what they started. Left behind, these keep running, and the GPU process can keep a core busy. The answer lists them as `ended`. The tree is read with `ps`, so this is on macOS and Linux. Other instances, and anything on the ports 47840 to 47842, never get this: there `quit` kills the app's process alone, as before.

### How the control channel works

The page has a driver, `src/driver.js`, that acts in XenoNet's own terms, such as "select this project". The desktop app's main process, `electron/control.js`, calls the driver and takes screenshots with `capturePage`. It sends real keyboard and mouse input through the window's own DevTools Protocol session, so a click goes through the map's own hit-testing.

Requests reach it as `POST /api/dev/control` on the instance's server, which relays them. That route exists only in XenoNet Dev, never in a packaged XenoNet. It has the API's protections: `127.0.0.1` only, the `Host` check, no cross-origin requests, and JSON only.

### Simulating rare states

`POST /api/dev/simulate {kind, a, b}`, which the `simulate` tool calls, puts sessions into rare states so you can check the visuals. It works on the dev server only.

- `needs-you`, `collision`, `stuck`, `wiki-write`, `turn-end` and `doctor` put sessions into those states. `doctor` is a problem the doctor found. `clear` undoes them. Without session ids, XenoNet picks the most recently active sessions.
- `actor-spawn`, `actor-edit`, `actor-message`, `actor-online` and `actor-offline` have an actor act on a node, so you can see it react and beam there. `a` is the actor, by default the first one that isn't you, and `b` is the node.
- In the demo and test instances, whose data is made up, `actor-request` has a made-up program ask to be an actor, so its card shows. `actor-needs-you` has an actor wait on you with a made-up question, until `clear`. `actor-tunnel` gives an actor a made-up connection through a tunnel, drawn as an open gate between it and the map, `actor-tunnel-down` shuts the gate, and `actor-read` sends a read through it.
- With `XENONET_HOSTS=1`, `host-add` shows another XenoNet on this machine as a host, with `a` as its id and `b` as its port. `host-offline`, `host-online` and `host-skew` change how a host stands. For `host-skew`, `b` is how many milliseconds the host's clock is off.
- The `net-…` kinds make up a [network across machines](../design/network.md), with other machines and their agents, for the Network panel, the attention strip and the message box. `net-none` is this machine on its own. `net-founder` has this machine start the network and be its gateway, with two more machines, one of them offline, and on laptop a session waiting on you with a question and another that seems stuck. `net-member` has this machine join one whose gateway is another. `net-gateway-down` has the gateway not answer, with what's for other machines waiting in the outbox. `net-ticket` has a ticket waiting, `net-mismatch` a machine that sees another list of the network's machines, and `net-remote-waiting` an agent on another machine waiting on you. For `net-founder` and `net-remote-waiting`, `a` is the machine you're at, and `b` `titles-only` has the other machines share titles only. For `net-founder`, `b` `relay` has laptop drawn from its own map's snapshot instead of its presence: this machine's own map, served as laptop's by a relay over a network inside the service, so its transcripts open through the window as another machine's do. While one runs, the made-up network answers the network's routes from the window, so Add a machine shows a line, and a message to one of its agents goes into its outbox while its machine is away. `clear` ends it. The `network` tool, and the driver's `network` and `messageAs`, read and drive the panel and the message box.

### Other ways in

- In dev builds, `window.__xenonet` holds the scene, the panels, the audio and the driver. `?raf-shim` in the page's address keeps rendering going in hidden browser tabs.
- Generic browser tools can attach too. Start XenoNet Dev with `XENONET_CDP_PORT=9333`, then point Playwright MCP or Chrome DevTools MCP at `http://127.0.0.1:9333`. Playwright MCP takes the address with `--cdp-endpoint`, and Chrome DevTools MCP with `--browserUrl`. Neither officially supports Electron, and neither can see inside the map's WebGL canvas, so prefer the tools above.

## Layout

XenoNet is one core with several ways in: the console, the desktop app, the HTTP API, the `xenonet` command and the MCP servers. The service, the command, the hooks and the MCP servers that XenoNet runs are the `xenonet` binary, built from `crates/`. `core/`, `server/`, `cli/` and `mcp/` are the same in JS: the contract the Rust is tested against, and the modules Desktop and the page import. The core holds the logic. Each of the others is a thin layer over it, which imports from `core/` and from no other layer. The one exception is `xenonet mcp`, which starts the MCP server in `mcp/`.

The console page imports only the core modules that have no Node in them, such as `themes.js`, `units.js`, `needs-you.js` and `apps.js`. The `xenonet` command and the MCP servers reach a running XenoNet through its HTTP API, with `core/client.js`. They reach XenoNet Dev through its control channel, with `core/control.js`.

### `core/`

`core/` is XenoNet's core in JS, which the Rust in `crates/` follows module by module.

| Files | What they cover |
| --- | --- |
| `apps.js` | The app registry: the apps XenoNet knows sessions of, and what's fixed about each (its folder, hook events, signing team, edit tools and what XenoNet can do with it). The page uses it too, and `node scripts/apps.mjs` writes the Rust's copy, `crates/xenonet-base/src/apps.json` |
| `claude.js`, `codex.js`, `grok.js`, `git.js`, `wiki.js`, `demo.js` | The sources XenoNet reads |
| `native/`, `numbat.js`, `reported-runs.js` | Runs of other apps: the experimental native readers, one for each app the registry gives `native` (the table in `native/index.js`, the contract each keeps in `native/reader.js`), numbat's records, and the runs actors report |
| `lines.js` | Transcript lines, as all three of XenoNet's readers take them |
| `vendor.js`, `drift.js`, `schema/` | The apps' data, read leniently against XenoNet's schemas, and the drift kept from it. The schemas themselves are in `schema/`. |
| `projects.js` | How the projects folder is read |
| `classify.js` | The Codex pass over the projects folder |
| `workspace.js` | Which folder the projects folder is |
| `live.js` | The transcript reader and its signals |
| `graph.js`, `state.js` | The map's data |
| `search.js`, `digest.js` | Search and digests |
| `redact.js`, `export-labels.js`, `frames.js` | What an export of the day may say (star names, the archetype), what the map's own labels show during one, and the frames posters and films come in |
| `identities.js` | Who sessions ran as |
| `claude-usage.js` | Claude plan usage |
| `config.js`, `dirs.js`, `paths.js`, `store.js` | Settings, and the places XenoNet keeps things |
| `statefile.js`, `migrate.js` | The versions of `state.json` |
| `blocks/` | The building blocks XenoNet's subsystems are made of: scanners, gatherers, tailers, normalizers and stores |
| `open.js` | Opening sessions in their apps |
| `engine/` | The engine that connects sessions to each other, and connecting the apps to it (`connect.js`, with `toml.js` for Grok Build's `config.toml`) |
| `procs.js` | Which process is on the other end of a connection to the engine |
| `actors/` | Actors, with the registry, registering and tokens, you, the observers of Hermes Agent, Grok Bot and Muse, remote actors' connections, account links, provider adapters and presence, and what each actor's work costs (`spend.js`) |
| `assistant/` | What remote actors read: the projection, the read service, the made-up fixture, page tokens, the log, credentials and tombstones, and the adapter's client (see [Remote actors](../design/cloud-assistants.md)) |
| `doctor/` | The doctor's checks |
| `stats.js`, `pricing.js` | Sessions' stats, and API prices by date |
| `units.js` | Money, tokens and durations as people read them. The page uses it too. |
| `needs-you.js` | Everything waiting on you as one queue, the longest wait first, with what answers each: the Needs you panel, the tray and `xenonet status` read it. The page uses it too. |
| `themes.js`, `themefiles.js` | Themes. The page uses `themes.js` too, and `themefiles.js` handles your own. |
| `script.js` | Running steps and scripts against XenoNet Dev |
| `client.js` | The client for a running XenoNet's API |
| `command.js` | The `xenonet` command's script on your PATH |
| `hosts/` | Other machines' XenoNets. It has the relay, the link to each host, merging their maps and keeping only what the schemas declare, each host's last snapshot, `hosts.json` and XenoNet's own ssh config, and the doctor's check for hosts that share a key. |
| `instances.js` | XenoNet Dev's instances |
| `control.js` | The client for XenoNet Dev's control channel |

### `server/`

`server/` is the HTTP API over the core, in `index.js`. `upstream.js` passes XenoNet Dev's reads to another XenoNet, read-only. The routes for actors, remote connections, drift, the engine, hosts, themes and the projects folder are in `routes/`. `assistant-endpoint.js` is the private endpoint remote actors' adapters reach, apart from the API. The server also serves the console page, with its Content-Security-Policy, from `static.js`.

### `src/`

`src/` is the console page.

| Files | What they cover |
| --- | --- |
| `scene.js`, `nodes.js` | Drawing the 3D map |
| `fx.js` | The textures, lines, sky and shaders the map uses |
| `looks/` | Drawing the stars, the floor, the sweep, the sonar, the lines between nodes and the actors' forms, in each theme's styles |
| `actors.js`, `actors-model.js` | The actors' layer |
| `hosts.js`, `hosts-draw.js`, `network-map.js` | Other machines' constellations: over SSH, and a network's machines, drawn from their presence |
| `framing.js` | Where the camera stands to frame the map |
| `hud.js`, `telemetry.js`, `actor-spend-view.js`, `alerts.js`, `needs-you.js`, `modals.js`, `palette.js`, `replay.js`, `terminal.js`, `identities.js` | The panels around the map. `actor-spend-view.js` has Token use's rows of spend by actor. |
| `export-panel.js`, `poster.js`, `poster-draw.js`, `mark.js` | The export panel, and posters of the day: the preview, the sizes, what an export reveals, and the mark |
| `connect.js` | The panel for connecting Claude Code, Codex and Grok Build |
| `background.js` | The Background service panel, and Desktop's offer of it during setup |
| `network.js`, `network-model.js` | The Network panel (Network settings, Add a machine) for a network across machines, and the words for it, which the attention strip shares |
| `doctor.js` | The panel for XenoNet's doctor |
| `stats.js` | The panel for a session's stats |
| `film.js`, `film-cards.js` | Films of the day, and their cards |
| `root.js` | The panel for changing the projects folder |
| `themes-ui.js`, `theme-switcher.js`, `theme-gallery.js` | Themes from the palette |
| `actors-panel.js`, `actor-inspector.js`, `actor-requests.js`, `gateway-panel.js`, `wake-panel.js`, `remote-panel.js`, `remote-views.js` | The Actors panel, an actor in the inspector, the programs asking to be one, the actor gateway, how actors are woken, and remote connections and their views in the panel |
| `ambient.js`, `sound-space.js` | Music and sound effects, and where on the map each sound comes from |
| `oplog.js` | What you do, kept as steps |
| `you.js` | What you do at the console, told to XenoNet with the window's key |
| `theme.js` | Applying the theme before anything is drawn |
| `driver.js` | Driving it all for the control channel |

### `electron/`

`electron/` is the desktop app.

| Files | What they cover |
| --- | --- |
| `main.js` | The window and the server process |
| `pty.js` | Shells |
| `tray.js`, `icons.js`, `preload.cjs` | The tray, the icons and the preload bridge |
| `trust.js` | The only page the app answers |
| `login.js` | Opening Desktop at login |
| `background.js` | Starting the service at login, through the `xenonet` command |
| `shellenv.js` | The login shell's `PATH` |
| `mac.js` | The macOS menu bar, the first run and the uninstaller |
| `theme-menu.js` | The Theme menu in the tray and the menu bar |
| `control.js`, `keys.js` | XenoNet Dev's control channel |
| `recorder.js` | Recording the window to an MP4 |

### `cli/`

`cli/` is the `xenonet` command. `xenonet.js` reads the arguments and loads one command from `commands/`, and `lib/` has its output helpers. Claude Code and Codex run the command too, as `xenonet mcp` and `xenonet hook`. A console on another machine runs `xenonet relay` over SSH.

### `mcp/`

`mcp/` has the MCP servers. `xenonet.js` has the coordination tools, for every session and actor on the machine. `console.js` has the xenonet-console tools, for operating installed Desktop and checking development copies. `assistant.js` has the six read-only tools a remote actor reads through.

### `scripts/`

`scripts/` has build and development tools:

- `check-contracts.mjs`
- `provider-drift.mjs` and `provider-drift/`, the daily check of new provider releases against the vendor schemas (see [Provider drift](provider-drift.md))
- `schemas.mjs`, for the schema files in `schemas/`
- `bench.mjs`, the benchmark
- `gpu-bench.mjs` (`npm run bench:gpu`), what the console costs the machine on a Mac: GPU time, frame time, and the CPU and memory of Desktop and the service, with the window at rest, turning, busy, behind another app, covered, minimized and hidden, beside the machine's GPU use without XenoNet
- `bench-history.mjs`, a made-up history of Claude Code and Codex sessions in a folder of its own, from a few hundred sessions to a few thousand, with long ones, subagents and files that keep growing, for measuring the service at scale
- `hook-snippet.mjs`
- `dev-install.mjs`
- `dist.mjs`
- `builder-hooks.mjs`, the build hooks
- `lab/`, the VM lab
- `demo/`, OpenCode sessions messaging each other through a XenoNet from the checkout, with the tools it gives them (see [OpenCode sessions messaging each other](opencode-messaging.md))

### `test/`

`test/` has the unit tests, and `test/e2e/` has the end-to-end tests.

### `crates/`

`crates/` is the Rust core, a crate per layer ([Rust core](../design/rust-core.md)), and builds the `xenonet` binary that XenoNet runs. `xenonet-node` puts the Rust behind the JS modules, so the JS tests check it too. Each ported module has a twin in `crates/xenonet-node/js/core/`, with the module's exports, backed by Rust. `npm run rust:build` builds the addon. `npm run test:rust` runs the test files listed in `crates/xenonet-node/parity.json` with every twin loaded in place of its module, and `npm run test:rust -- --all` tries every test file that way. `npm test` still runs the JS.
