# XenoNet components

XenoNet has three installed parts:

| Component | What it does |
| --- | --- |
| Desktop | Shows the map, panels and terminals. |
| Service | Tracks and coordinates sessions, stores state and runs terminal processes. |
| Tools | Provides the `xenonet` command, the hooks sessions run and both MCP servers. |

The bundle installer includes all three. Service and Tools also have separate downloads for Windows x64, macOS arm64 and x64, and Linux x64. Each includes its Node runtime. You do not need to install Node.

Closing or updating Desktop leaves the service, terminals and connected MCP processes running. Reopening Desktop restores terminal tabs and recent output. Terminal output stays in memory. A service restart ends its terminals. If terminals are running, XenoNet downloads the service update but leaves it uninstalled. Close the terminals and choose Update again to install it.

## Install without Desktop

Download and extract the Service and Tools archives for your operating system and CPU from the same GitHub release. Compare their SHA-256 hashes with `SHA256SUMS.txt`.

From each extracted folder, run:

```sh
./node cli/xenonet.js setup
```

On Windows, use `./node.exe cli/xenonet.js setup`. Either component can be installed first. Tools installs the `xenonet` command. Start the service with `xenonet service start`; `xenonet service status` reports its version and running terminals. `xenonet service stop` refuses while terminals are running. `--force` stops them too.

The service runs as your account, independently of Desktop. It stays running until stopped or you sign out. Opening Desktop starts it when needed. This release does not register an operating-system service or a separate login task.

### The console in a browser

The service serves the console too: open http://127.0.0.1:47840 in a browser on the same computer. The map, panels, alerts, search and stats work as in Desktop. What only XenoNet's own window may do, such as allowing an actor, storing a key or controlling Spotify, says so in a browser; do it with the `xenonet` command (`xenonet connect`, `xenonet actor`, `xenonet decide key`) or in Desktop. Terminals, tray notifications and Desktop's updater need Desktop; `xenonet update service` and `xenonet update tools` update the rest.

## MCP servers

`xenonet mcp` runs the coordination MCP server. With it, each session can see the others, message them and claim files.

`xenonet mcp console` runs the console MCP server. With it, an agent can launch installed Desktop, navigate the map, inspect panels, use terminals and take screenshots. The installed Tools package targets installed XenoNet by default. `xenonet console --instance installed <steps>` offers the same control from a terminal.

`xenonet mcp assistant --grant <grant>` runs the read-only server for a [remote actor](cloud-assistants.md) you paired. A tunnel client runs it, not Claude Code or Codex, and it reaches the service only through a private endpoint of its own, with that grant's credential; it never gets the Tools credential.

Use `Connect Claude Code and Codex to XenoNet` in the palette, or `xenonet connect`, to review both MCP servers and add them to Claude Code and Codex. Their configuration points to a stable launcher outside Desktop. A new Tools version applies to new commands and new MCP connections. MCP processes already running keep their code and runtime until their client reconnects.

Console MCP stays connected while Desktop restarts. Window actions report that Desktop is unavailable until it returns. Coordination continues through the service. Simulations, arbitrary JavaScript evaluation and raw DevTools commands remain available only in development instances. Actions marked as yours alone in the app keep that restriction.

## Updates

Desktop's **Updates** panel checks all three components. Desktop downloads contain only Desktop. Service and Tools can be updated separately in that panel or from the command line:

```sh
xenonet update --check
xenonet update tools
xenonet update service
```

Downloads are checked against the release manifest's size and SHA-256 hash. Each component is installed into a new version directory. The selected version changes only after validation. Old versions are retained so running MCP processes can finish and a failed service startup can return to the previous version.

Unsigned component archives rely on GitHub's HTTPS release channel and checksums; they do not have a separate publisher signature. Stable releases are the only automatic update channel. Desktop's existing Windows signing and macOS notarization requirements still apply.

## Compatibility

Desktop, Service and Tools have separate release versions. Their interface versions are independent: service API 1, console API 1, and assistant API 1 for remote actors' adapters. Connections negotiate a shared version; incompatible components refuse to connect with an update instruction. HTTP clients send `x-xenonet-api-version: 1`. Older clients that omit it retain the original API behavior.

The service owns persistent data. Current component packages declare data schema 1. A future schema change must define its migration and rollback policy before changing that number. New API fields must be optional for older clients, and features must check capabilities before using a new interface.

Component files live under XenoNet's state folder in `runtime/components`. The private service connection file is restricted to the current account. Desktop serves its own frontend and proxies API calls to the service; its assets never come from a different service release. It passes on only its own window's API calls. Other programs call the service directly, so the service can see which program sent each request.

On macOS, the service treats the connected Desktop as XenoNet's own app, so the window can report what you do there. It hands the window key only to a Desktop it can see is an app's own executable, not an agent, the `xenonet` command or a script.
