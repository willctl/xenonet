# Development

## Commands

```bash
npm run dev       # XenoNet Dev's server and hot-reloading UI at http://127.0.0.1:47842
npm run demo      # the same, with made-up data, at http://127.0.0.1:47841
npm run app       # the desktop app (Electron) from source, as XenoNet Dev
npm run dev:install  # macOS: XenoNet Dev.app in ~/Applications, running this checkout
npm test          # unit tests
npm run test:e2e  # end-to-end tests, on a hidden XenoNet Dev with made-up data
node cli/xenonet.js console launch shot:overview  # drive XenoNet Dev from a terminal (see below)
npm run lab -- status  # macOS: the VM lab, Linux VMs to try other machines in (see below)
npm run dist      # the installers for this OS, in release/ (dist:win, dist:mac)
```

## XenoNet Dev

The desktop app run from source is XenoNet Dev. It has its own settings, port, layout and search index, so it runs beside an installed XenoNet without writing over its files. Its port is 47842. The first run copies the installed XenoNet's layout. `npm run dev`, the server alone, uses XenoNet Dev's port and folders too.

Connecting Claude Code and Codex from XenoNet Dev adds its own entries beside the installed XenoNet's, with its MCP server named `xenonet-dev`, and leaves the installed XenoNet's entries alone. Disconnecting takes away only its own. Whichever XenoNet connects next clears entries that point at a checkout that's gone. Terminals XenoNet opens don't pass on its own `XENONET_*` settings, so `npm run app` started in one runs as XenoNet Dev, not on the installed XenoNet's port.

XenoNet Dev picks up edits as you save them:

- The UI hot-reloads.
- A change under `core/` or `server/` restarts the server.
- A change under `electron/` relaunches XenoNet Dev.app. If you started XenoNet Dev with `npm run app`, it prints a reminder to restart instead.

Vite keeps its prebundled dependencies in each checkout's `.xenonet/vite`, which git ignores. So worktrees that link `node_modules` don't share them.

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

- **The page against real sessions.** On the other computer, `XENONET_UPSTREAM=http://127.0.0.1:47840 npm run dev` serves the page from the checkout and passes its reads to the installed XenoNet (`server/upstream.js`). Only reads go through: anything that would change something there is refused, so a page in development can look but never act. Add `--demo` to keep the dev server's own service on made-up data. `XENONET_UPSTREAM` takes only an address on that computer, a forwarded port, and the installed service ignores it.
- **Sessions talking to XenoNet Dev.** On the sessions' computer, `xenonet connect --here --port 47842` adds XenoNet Dev's MCP server and hooks to Claude Code's and Codex's settings beside the installed XenoNet's, through that computer's own launcher. Asking XenoNet Dev itself would write the other computer's settings. While the forward is down, its hooks do nothing. `xenonet disconnect --port 47842` takes them out again.

What acts on the sessions' computer (opening a session in its app, handoffs, terminals in its folders, its credential store) stays with the installed XenoNet there.

## Building

- `npm run dist:dir` builds only the app, such as `release/win-unpacked/XenoNet.exe` or `release/mac-arm64/XenoNet.app`. On Windows, `npm run shortcut` puts a desktop shortcut to it.
- macOS builds are ad-hoc signed, and the package is unsigned. The build stops if a certificate signs either one, because a certificate carries its owner's name. To sign on purpose, with a Developer ID, set `XENONET_SIGN=1`. The build also stops if a packaged file contains your home folder's path.
- The terminal's native module ships one npm package per platform and CPU. The build fetches the one each app needs, so one Mac builds both the Apple silicon and the Intel app.
- The icon is drawn in code, in `electron/icons.js`, at every size from 16 to 256 pixels, with a macOS cut on Apple's icon grid. `npm run icon` regenerates it, along with the installer art for both platforms.

[Install](install.md) says how to install what you build.

## Driving XenoNet from agents, scripts and tests

Installed XenoNet and XenoNet Dev both have a control channel, so an agent or a script can launch Desktop, act on it and see it the way you would. Development copies also offer simulation and evaluation for tests. Use the demo to check a change to XenoNet. Agents can operate the installed console for everyday work. [Components](components.md) explains the production MCP setup.

The control channel operates the console. The engine that connects sessions to each other is a separate MCP server, `xenonet`, in `mcp/xenonet.js`. See [Sessions working together](agents.md).

- Agents can use the control channel through the `xenonet-console` MCP server's tools, including navigation, search, screenshots, terminals, themes and updates. Installed Tools omit `eval` and `simulate`, which are only for development. `actors` includes the programs waiting for an Allow, and how each token stands. `feed` is the activity feed. `mcp_info` opens a shared system's metadata, tools and context catalog without executing tools. `environment` changes the surroundings independently of the theme.
- `.mcp.json` registers these tools for Claude Code, which asks you once to approve them. `.codex/config.toml` registers them for Codex, in a trusted project. Start Claude Code or Codex in the repo's root folder.
- From a terminal, `xenonet console <steps>` runs the same actions one after another, for example `xenonet console launch select:atlas-api shot:atlas`. `npm run drive -- <steps>` does the same. `xenonet console --help` lists the steps.
- XenoNet Dev keeps a log of what you do in it, as the same steps. Selecting a node, a palette command, a session's stats, a theme and a film all go in the log. `xenonet script record --instance dev > tour.json` saves it, and `xenonet script run tour.json` plays it back, in any instance, shown or hidden.
- A script is a JSON list of steps, written as strings such as `"select:atlas-api"`, or as objects such as `{ "op": "select", "args": [...] }`. Any driver method is a step. For anything else, a script can be a JS module whose default export gets the instance and a `step` function.
- `npm run test:e2e` drives a hidden XenoNet Dev with the demo's data through the same channel, in about five minutes. Run it before you send a change to the UI. A failing test saves a screenshot of what it saw to `.xenonet/`.
- `npm run test:real` checks, on a real Mac, that Grok Bot and Muse message each other through XenoNet Dev (the dev instance), through the actor gateway, or with `XENONET_REAL_ROUTE=local` from commands their apps run. It needs both apps signed in, you at XenoNet's window to set up the gateway and connect each, and someone to ask Grok Bot and Muse to do what it prints. It only reads XenoNet's state. `XENONET_REAL_PORT`, `XENONET_REAL_WORD` and `XENONET_REAL_MINUTES` change where it looks, the word the messages carry and how long it waits.
- Scripts import `core/control.js`, which all of the above use.

### Waiting and screenshots

Every action waits until the view has settled, then returns the new state, so nothing sleeps for a fixed time. The view has settled once the layout has cooled, the camera has stopped and any gathering has arrived.

A screenshot waits the same way. It also holds every ambient animation still, including the sky, the sonar, the sweep, sparks and CSS animations, so two shots of the same state match. Screenshots work while the window is covered, unfocused or hidden. They land in `.xenonet/`, which git ignores.

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

`quit` returns once XenoNet Dev has exited.

### How the control channel works

The page has a driver, `src/driver.js`, that acts in XenoNet's own terms, such as "select this project". The desktop app's main process, `electron/control.js`, calls the driver and takes screenshots with `capturePage`. It sends real keyboard and mouse input through the window's own DevTools Protocol session, so a click goes through the map's own hit-testing.

Requests reach it as `POST /api/dev/control` on the instance's server, which relays them. That route exists only in XenoNet Dev, never in a packaged XenoNet. It has the API's protections: `127.0.0.1` only, the `Host` check, no cross-origin requests, and JSON only.

### Simulating rare states

`POST /api/dev/simulate {kind, a, b}`, which the `simulate` tool calls, puts sessions into rare states so you can check the visuals. It works on the dev server only.

- `needs-you`, `collision`, `stuck`, `wiki-write`, `turn-end` and `doctor` put sessions into those states. `doctor` is a problem the doctor found. `clear` undoes them. Without session ids, XenoNet picks the most recently active sessions.
- `actor-spawn`, `actor-edit`, `actor-message`, `actor-online` and `actor-offline` have an actor act on a node, so you can see it react and beam there. `a` is the actor, by default the first one that isn't you, and `b` is the node.
- In the demo and test instances, whose data is made up, `actor-request` has a made-up program ask to be an actor, so its card shows. `actor-needs-you` has an actor wait on you with a made-up question, until `clear`. `actor-tunnel` gives an actor a made-up connection through a tunnel, drawn as an open gate between it and the map, `actor-tunnel-down` shuts the gate, and `actor-read` sends a read through it.
- With `XENONET_HOSTS=1`, `host-add` shows another XenoNet on this machine as a host, with `a` as its id and `b` as its port. `host-offline`, `host-online` and `host-skew` change how a host stands. For `host-skew`, `b` is how many milliseconds the host's clock is off.

### Other ways in

- In dev builds, `window.__xenonet` holds the scene, the panels, the audio and the driver. `?raf-shim` in the page's address keeps rendering going in hidden browser tabs.
- Generic browser tools can attach too. Start XenoNet Dev with `XENONET_CDP_PORT=9333`, then point Playwright MCP or Chrome DevTools MCP at `http://127.0.0.1:9333`. Playwright MCP takes the address with `--cdp-endpoint`, and Chrome DevTools MCP with `--browserUrl`. Neither officially supports Electron, and neither can see inside the map's WebGL canvas, so prefer the tools above.

## Layout

XenoNet is one core with several ways in: the console, the desktop app, the HTTP API, the `xenonet` command and the MCP servers. The core holds the logic. Each of the others is a thin layer over it, which imports from `core/` and from no other layer. The one exception is `xenonet mcp`, which starts the MCP server in `mcp/`.

The console page imports only the core modules that have no Node in them, `themes.js` and `units.js`. The `xenonet` command and the MCP servers reach a running XenoNet through its HTTP API, with `core/client.js`. They reach XenoNet Dev through its control channel, with `core/control.js`.

### `core/`

`core/` is XenoNet itself, in Node.

| Files | What they cover |
| --- | --- |
| `claude.js`, `codex.js`, `git.js`, `wiki.js`, `demo.js` | The sources XenoNet reads |
| `lines.js` | Transcript lines, as all three of XenoNet's readers take them |
| `vendor.js`, `drift.js`, `schema/` | The apps' data, read leniently against XenoNet's schemas, and the drift kept from it. The schemas themselves are in `schema/`. |
| `projects.js` | How the projects folder is read |
| `classify.js` | The Codex pass over the projects folder |
| `workspace.js` | Which folder the projects folder is |
| `live.js` | The transcript reader and its signals |
| `graph.js`, `state.js` | The map's data |
| `search.js`, `digest.js` | Search and digests |
| `identities.js` | Who sessions ran as |
| `claude-usage.js` | Claude plan usage |
| `config.js`, `dirs.js`, `paths.js`, `store.js` | Settings, and the places XenoNet keeps things |
| `statefile.js`, `migrate.js` | The versions of `state.json` |
| `blocks/` | The building blocks XenoNet's subsystems are made of: scanners, gatherers, tailers, normalizers and stores |
| `open.js` | Opening sessions in their apps |
| `engine/` | The engine that connects sessions to each other |
| `procs.js` | Which process is on the other end of a connection to the engine |
| `actors/` | Actors, with the registry, registering and tokens, you, the observers of Hermes Agent, Grok Bot and Muse, and remote actors' connections, account links, provider adapters and presence |
| `assistant/` | What remote actors read: the projection, the read service, the made-up fixture, page tokens, the log, credentials and tombstones, and the adapter's client (see [Remote actors](../design/cloud-assistants.md)) |
| `doctor/` | The doctor's checks |
| `stats.js`, `pricing.js` | Sessions' stats, and API prices by date |
| `units.js` | Money, tokens and durations as people read them. The page uses it too. |
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
| `hosts.js`, `hosts-draw.js` | Other machines' constellations |
| `framing.js` | Where the camera stands to frame the map |
| `hud.js`, `telemetry.js`, `alerts.js`, `modals.js`, `palette.js`, `replay.js`, `poster.js`, `terminal.js`, `identities.js` | The panels around the map |
| `connect.js` | The panel for connecting Claude Code and Codex |
| `doctor.js` | The panel for XenoNet's doctor |
| `stats.js` | The panel for a session's stats |
| `film.js` | Films of the day |
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
| `login.js` | Starting at login |
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
- `hook-snippet.mjs`
- `dev-install.mjs`
- `dist.mjs`
- `builder-hooks.mjs`, the build hooks
- `lab/`, the VM lab

### `test/`

`test/` has the unit tests, and `test/e2e/` has the end-to-end tests.
