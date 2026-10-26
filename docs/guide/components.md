# XenoNet components

XenoNet has three installed parts:

| Component | What it does |
| --- | --- |
| Desktop | Shows the map, panels and terminals. |
| Service | Tracks and coordinates sessions, stores state and runs terminal processes. |
| Tools | Provides the `xenonet` command, the hooks sessions run and both MCP servers. |

The bundle installer includes all three. Service and Tools also have separate downloads for Windows x64, macOS arm64 and x64, and Linux x64. Each is the `xenonet` binary for that system, in `bin/`. You do not need to install Node.

On a Mac there's a fourth, the tray: XenoNet Tray.app, a menu bar app that shows whether XenoNet runs and what needs you while Desktop is closed. It's one download for both Mac CPUs, `XenoNet-tray-<version>-darwin-universal.tar.gz`, with a version of its own. The disk image brings it into XenoNet's runtime folder, and `xenonet service install` puts it in `~/Applications` and starts it at login. With the background service already on, run `xenonet service install` again after updating the Tools to get a newer tray. See [The tray on macOS](always-on.md#the-tray-on-macos).

Closing or updating Desktop leaves the service, terminals and connected MCP processes running. Reopening Desktop restores terminal tabs and recent output. Terminal output stays in memory. A service restart ends its terminals. If terminals are running, XenoNet downloads the service update but leaves it uninstalled. Close the terminals and choose Update again to install it.

## Install without Desktop

Download and extract the Service and Tools archives for your operating system and CPU from the same GitHub release. Compare their SHA-256 hashes with `SHA256SUMS.txt`.

From each extracted folder, run:

```sh
./bin/xenonet setup
```

On Windows, use `bin\xenonet.exe setup`. Either component can be installed first. Tools installs the `xenonet` command. Start the service with `xenonet service start`; `xenonet service status` reports its version and running terminals. `xenonet service stop` refuses while terminals are running. `--force` stops them too.

The service runs as your account, independently of Desktop. It stays running until stopped or you sign out. Opening Desktop starts it when needed. `xenonet service install` has it start at login instead, and start again if it stops unexpectedly, under launchd, systemd or Task Scheduler. `xenonet service uninstall` turns that off. See [Keeping the service running](always-on.md).

A machine that only serves as a host for another machine's console needs the Service and Tools, `xenonet setup` from each, and `xenonet service install`. On Linux, `loginctl enable-linger $USER` starts it at boot, before anyone logs in.

### The console in a browser

The service serves the console too: open http://127.0.0.1:47840 in a browser on the same computer. The map, panels, alerts, search and stats work as in Desktop. What only XenoNet's own window may do, such as allowing an actor, storing a key or controlling Spotify, says so in a browser; do it with the `xenonet` command (`xenonet connect`, `xenonet actor`, `xenonet decide key`) or in Desktop. Terminals, tray notifications and Desktop's updater need Desktop; `xenonet update service` and `xenonet update tools` update the rest.

## MCP servers

`xenonet mcp` runs the coordination MCP server. With it, each session can see the others, message them and claim files.

`xenonet mcp console` runs the console MCP server. With it, an agent can launch installed Desktop, navigate the map, inspect panels, use terminals and take screenshots. The installed Tools package targets installed XenoNet by default. `xenonet console --instance installed <steps>` offers the same control from a terminal.

`xenonet mcp assistant --grant <grant>` runs the read-only server for a [remote actor](cloud-assistants.md) you paired. A tunnel client runs it, not Claude Code or Codex, and it reaches the service only through a private endpoint of its own, with that grant's credential; it never gets the Tools credential.

Use `Connect your agents to XenoNet` in the palette, or `xenonet connect`, to review both MCP servers and add them to Claude Code, Codex and Grok Build. Their configuration points to a stable launcher outside Desktop. A new Tools version applies to new commands and new MCP connections. MCP processes already running keep their code and runtime until their client reconnects.

Console MCP stays connected while Desktop restarts. Window actions report that Desktop is unavailable until it returns. Coordination continues through the service. Simulations, arbitrary JavaScript evaluation and raw DevTools commands remain available only in development instances. Actions marked as yours alone in the app keep that restriction.

## Updates

Desktop's **Updates** panel checks all three components. Desktop downloads contain only Desktop. Service and Tools can be updated separately in that panel or from the command line:

```sh
xenonet update --check
xenonet update tools
xenonet update service
```

The tray isn't in the Updates panel or `xenonet update` yet. A newer disk image replaces it, and `xenonet service install` copies the new one to `~/Applications`.

Downloads are checked against the release manifest's size and SHA-256 hash. Each component is installed into a new version directory. The selected version changes only after validation. Old versions are retained so running MCP processes can finish and a failed service startup can return to the previous version.

The service runs from `runtime/components/service/active`, a copy of the selected version's directory, not from the version directory itself. A first install, a Service update and a rollback each copy the version there while the service is stopped, and `active/.from` names the version it is. So the running `xenonet` keeps one path from version to version. A firewall that lets it accept connections, as a [network's gateway](network.md#starting-a-network) on a LAN or tailnet address needs, keeps letting it in after an update. When `active` isn't a copy of the selected version, because no update has made it yet or the copy failed, the service runs from the version's own directory.

Unsigned component archives rely on GitHub's HTTPS release channel and checksums; they do not have a separate publisher signature. Stable releases are the only automatic update channel. Desktop's existing Windows signing and macOS notarization requirements still apply.

## Upgrading from 0.3.0

In 0.3.0, the Service and Tools packages were a `node` of their own with XenoNet's JS beside it. Now each is the `xenonet` binary in `bin/`. The 0.3.0 updater expects a `node` in each package, so `xenonet update` from 0.3.0, and the Updates panel in 0.3.0 Desktop, stop with an error about `node`. Update Desktop instead, or run the new packages' own setup:

- The bundle, XenoNet Setup on Windows or the disk image on a Mac, installs its own Service and Tools when Desktop starts. It replaces any that has no `xenonet` binary, as it would install a missing one. A bundle never replaces a component that has its binary.
- A Desktop-only update, from the Windows Updates panel or the Mac ZIP, downloads the Service and Tools from the latest release when Desktop starts.
- Without Desktop, run `./bin/xenonet setup` from the extracted Tools archive, then from the Service archive.

The Tools switch at once. The service switches as a service update does:

- XenoNet asks the running 0.3.0 service to stop over its component connection, starts the new one, and checks that it answers as the new version.
- While a terminal is open in the 0.3.0 service, XenoNet leaves it running. Desktop connects to it and says so. Close the terminals, then quit and reopen XenoNet. Setup reports the service update as staged. Run it again once the terminals are closed.
- If the new service doesn't start, XenoNet starts the 0.3.0 service again with its own `node`, and Desktop says why. If Desktop can't download the new components, it runs with the 0.3.0 ones. Either way, quit and reopen XenoNet to try again.

0.3.0 registered nothing to start at login, so there's no registration to change. Its version folders stay in `runtime/components` for MCP processes still running from them and for that rollback.

The new service reads 0.3.0's state where it is. `engine.sqlite` goes from version 7 to 8, which adds the tables and columns for [a network across machines](network.md). XenoNet copies it to `engine.v7.sqlite` first and deletes the copy a week later. If the update is rolled back, the 0.3.0 service leaves the newer file's shape alone and reads the rest as before. `state.json` and the search index keep their versions, and the data schema stays 1.

Hooks and MCP entries that 0.3.0 wrote run the launcher's `node` and `cli/xenonet.js`. XenoNet keeps that script current, and it now runs the `xenonet` binary, so they keep working. `xenonet connect` replaces them with entries that run the binary.

## Compatibility

Desktop, Service and Tools have separate release versions. Their interface versions are independent: service API 1, console API 1, and assistant API 1 for remote actors' adapters. Connections negotiate a shared version; incompatible components refuse to connect with an update instruction. HTTP clients send `x-xenonet-api-version: 1`. Older clients that omit it retain the original API behavior.

The service owns persistent data. Current component packages declare data schema 1. A future schema change must define its migration and rollback policy before changing that number. New API fields must be optional for older clients, and features must check capabilities before using a new interface.

Component files live under XenoNet's state folder in `runtime/components`. The private service connection file is restricted to the current account. Desktop serves its own frontend and proxies API calls to the service; its assets never come from a different service release. It passes on only its own window's API calls. Other programs call the service directly, so the service can see which program sent each request.

On macOS, the service treats the connected Desktop as XenoNet's own app, so the window can report what you do there. It hands the window key only to a Desktop it can see is an app's own executable, not an agent, the `xenonet` command or a script. The tray gets a key of its own the same way, only when it's `XenoNet Tray.app` in `/Applications` or `~/Applications` with no agent above it, and each of its requests must come from you as well. Neither key is written to disk.
