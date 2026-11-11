# How it works

XenoNet helps actors, people and bots, take an intent to an outcome. The work runs on hosts (your machines), as identities (the accounts it's done as), in running sessions (Claude Code, Codex, Grok Build and other agents), and each actor works only within the grant it holds. A background service on each machine does this: it reads what the agent apps write, keeps one store, decides every request against the grants, and links your machines into a network. The console, the `xenonet` command and the MCP servers all talk to that service. [goals.md](../design/goals.md) says what XenoNet is for, and [direction.md](../design/direction.md) where the code stands.

This page lists what the service reads, where it keeps its files and the environment variables it reads.

XenoNet reads Claude Code's, Codex's and Grok Build's files. It acts only when you ask it to open a session, hand one off, start a terminal or rename a session. Renaming is the one time it writes to the apps' session files, and connecting your agents is the one time it changes their settings.

## Where the data comes from

| Data | Source | Stability |
| --- | --- | --- |
| Projects and repo status | A scan of `~/Projects`, described in [Finding projects](projects.md), and `git status --porcelain -b` when a repo's head, refs or config change or a session works in it (see [Repo status](privacy.md#repo-status)) | Yours |
| Codex sessions and transcripts | `codex app-server` JSON-RPC, started from the ChatGPT desktop app's bundled binary or the Codex CLI | Official API, labelled experimental |
| Claude Code sessions and transcripts | Its transcript files in `~/.claude/projects`, or under `CLAUDE_CONFIG_DIR`, read as the Agent SDK's `listSessions` and `getSessionMessages` read them | Internal format, best effort |
| Grok Build sessions and transcripts | Each session's folder under `~/.grok/sessions`, or `GROK_HOME`'s: its `summary.json`, its `updates.jsonl` of ACP session updates, and its `usage.json` of tokens and cost. See [Grok Build in XenoNet](../design/grok-build.md). | `summary.json` is in Grok's guide; the rest is undocumented, best effort |
| Live activity, tokens, context, subagents and edits | The end of each app's transcript files, read as the system's file events say they grow, with a walk of the folders at start and every 10 minutes for what the events missed | Internal format, best effort |
| Codex rate limits | `token_count` events in Codex rollouts | Undocumented, optional |
| Claude plan usage | Anthropic's `/api/oauth/usage`, the endpoint Claude Code's `/usage` reads. XenoNet asks every 5 minutes, with Claude Code's sign-in. It reads the sign-in from the "Claude Code-credentials" keychain item on macOS, or else from `~/.claude/.credentials.json`. Claude desktop's `plan-usage-history.json` fills in when it's newer. | Undocumented, optional |
| Codex desktop projects, and Claude desktop stars, archives and worktrees | `~/.codex/state_5.sqlite`, and Claude desktop's session files | Undocumented, optional |
| Identities | `~/.claude.json`'s signed-in account, Claude desktop's session folders, Remote Control transcripts, the claims in `~/.codex/auth.json`'s ID token, gh's `hosts.yml`, and `git config` in each repo. See [Identities](identities.md). | Undocumented, optional |
| Actors | Hermes Agent's state in `~/.hermes`, or in `HERMES_HOME`, for metadata only. The processes Grok Bot's app runs, and the app's version. Whether Muse's app runs, and its version. What the programs you allowed report. See [Actors](actors.md). | Undocumented, optional. Your programs report through XenoNet's own API. |
| Shared context | Xenolith's folder of pages, found as Xenolith finds it, or the folder in `XENONET_WIKI_DIR`. See [Shared context](shared-context.md). | Yours |
| Search | A local SQLite full-text index in the cache folder. When a newer XenoNet reads transcripts differently, it indexes part of them again. | Rebuildable |
| Estimated costs | Anthropic's and OpenAI's published API prices, by the date each took effect, in `crates/xenonet-model/src/usage/pricing.rs`, checked 2026-10-03. These are standard tier prices. Batch and data residency aren't counted. A session's stats and Token use's estimate for today use them, and so does [what each actor's work costs](actors.md#what-each-actors-work-costs). Grok Build's cost is what Grok recorded in its `usage.json`, which it does only with an API key; otherwise its cost shows as unknown. | Updated by hand. A model released later shows "no price" until it's added. |
| OpenRouter spending | OpenRouter's `GET /api/v1/key`, with the key you gave XenoNet for [decision models](decisions.md), at most once a minute while a window is open | Official API, optional |
| This computer's load | For the CPU and memory, the system's own counters: `sysctl` and the Mach host statistics on a Mac, `/proc` on Linux, and each core's times on Windows. For the GPU: NVIDIA's `nvidia-smi`, left running in its loop mode; on a Mac, `ioreg`'s IOAccelerator statistics; on Linux, the amdgpu driver's files in `/sys`; on Windows without `nvidia-smi`, Windows' GPU counters through PowerShell. Read every two seconds only while the panel shows and the window can be seen; the GPU reader stops a minute after. In a network, also every 5 seconds while a window host's console watches this machine, unless it doesn't share its load ([What crosses](network.md#what-crosses-and-what-doesnt)). | The system's own, stable |
| Summaries, and Codex's picks of projects | `codex exec --ephemeral -s read-only`, only when you press the button. It runs in a Codex home of its own, which links your `auth.json` and nothing else, so your plugins, hooks, MCP servers and `AGENTS.md` stay out of it. Its shell and web search are off. | Official CLI, leaves no session behind |
| Renaming a session | Claude Code: a custom title added to the transcript, the line the Agent SDK's `renameSession` adds, as Claude desktop does when you rename there. Claude desktop: the title in its own record of the session, which it reads when it starts. Codex: `thread/name/set` on `codex app-server`, which saves the name in Codex's index, state database and rollout. | Codex: official API. Claude Code: the Agent SDK's format. Claude desktop: undocumented, and it shows the new name the next time it starts. |
| Open in app and handoff | `codex://threads/<id>`, `codex://threads/new?prompt=&path=`, `claude://code/continue?session=`, `claude://code/new?folder=&q=` and `claude://code/needs-input` | Codex's links are its own. Claude's come from its app bundle and are undocumented. |

## Drift

If an undocumented source changes shape, only the extras it adds disappear. XenoNet reads each app's data against a schema of what it expects. It keeps what differs as drift, by source, in `drift.json` in its cache folder. Drift holds shapes and types, never values.

The doctor warns about drift that's new since you last looked. `xenonet drift` lists it and marks it seen. `xenonet drift --all` lists everything kept. `xenonet drift --exit-code` is for scripts. It exits 1 when there's new drift, and leaves the drift unmarked, so a script goes on exiting 1 until you look.

## Checking the adapters after an update

After you update Claude Code or Codex, the doctor warns about drift the new version brought, and `xenonet drift` lists it, by source.

## Where XenoNet keeps files

XenoNet never adds a dot folder to your home folder. XDG variables win when they're set. Otherwise, XenoNet uses the platform's standard places.

| What | Windows | macOS | Override |
| --- | --- | --- | --- |
| The app | `%LOCALAPPDATA%\Programs\XenoNet` | `/Applications/XenoNet.app` | The installer |
| XenoNet's one store, `xenonet.sqlite`: the layout, dismissed alerts, your rules and Codex's picks for the projects folder, your names and pins for identities, and the theme; projects and their checkouts, and each session's `agt_` id; what sessions said to each other and their claims; [actors'](actors.md) events, targets, grants' scopes and revocation ids (never the tokens); live delivery's settings, runs and audit log; [A2A](actors.md#agents-that-speak-a2a) tasks; your networks, their members and each machine's feed; and your person chain. Only your account can read it. A build refuses a store another build wrote and leaves it on disk: set the machine up again ([When something goes wrong](troubleshooting.md)). | `%LOCALAPPDATA%\XenoNet\xenonet.sqlite` | `~/Library/Application Support/XenoNet/xenonet.sqlite` | `XENONET_STATE_DIR`, `XDG_STATE_HOME` |
| Your [themes](themes.md) | `%LOCALAPPDATA%\XenoNet\themes\` | `~/Library/Application Support/XenoNet/themes/` | `XENONET_STATE_DIR`, `XDG_STATE_HOME` |
| Your keys on this machine: your root key (`root.json`), and your device key and this machine's signing key where they are files (`device-sign.key`, `machine-sign.key`). On a Mac both are in the Secure Enclave and on Windows the machine's is in the TPM, so no file holds them. Your person chain and the roots actors' tokens are made from are in the store. See [Authorization](../design/authorization.md). | `%LOCALAPPDATA%\XenoNet\identity\` | `~/Library/Application Support/XenoNet/identity/` | `XENONET_STATE_DIR`, `XDG_STATE_HOME` |
| This machine's transport key in a [network](network.md) (`host.key`), and, on the gateway's machine, the network's queues (`gateway.sqlite`) | `%LOCALAPPDATA%\XenoNet\network\` | `~/Library/Application Support/XenoNet/network/` | `XENONET_STATE_DIR`, `XDG_STATE_HOME` |
| The API credential each start writes for this account's programs, `api-<port>.cred`, new at every start | `%LOCALAPPDATA%\XenoNet\runtime\` | `~/Library/Application Support/XenoNet/runtime/` | `XENONET_RUNTIME_DIR`, `XDG_STATE_HOME` |
| The search index, which can be rebuilt | `%LOCALAPPDATA%\XenoNet\Cache\search.sqlite` | `~/Library/Caches/XenoNet/search.sqlite` | `XENONET_CACHE_DIR`, `XDG_CACHE_HOME` |
| [Drift](#drift), which is what differs in the apps' data from what XenoNet expects, kept as shapes | `%LOCALAPPDATA%\XenoNet\Cache\drift.json` | `~/Library/Caches/XenoNet/drift.json` | `XENONET_CACHE_DIR`, `XDG_CACHE_HOME` |
| Each [network](network.md) machine's last map snapshot, so an offline machine stays on the map | `%LOCALAPPDATA%\XenoNet\Cache\hosts\` | `~/Library/Caches/XenoNet/hosts/` | `XENONET_CACHE_DIR`, `XDG_CACHE_HOME` |
| Window settings, such as mute and terminal height | `%APPDATA%\XenoNet` | `~/Library/Application Support/XenoNet` | |
| Posters | `Pictures\XenoNet` | `~/Pictures/XenoNet` | `XENONET_CAPTURES_DIR` |
| Replay clips and films | `Videos\XenoNet` | `~/Movies/XenoNet` | `XENONET_CAPTURES_DIR` |

On Linux, these go in the XDG defaults, `~/.local/state` and `~/.cache`.

## Configuration

| Environment variable | Default |
| --- | --- |
| `XENONET_PORT` | `47840`. Demo mode uses `47841`, and XenoNet Dev uses `47842`. |
| `XENONET_PROJECTS_DIR` | `~/Projects` |
| `XENONET_PROJECTS_DEPTH` | `3`, how many levels below the projects folder the scan looks for projects |
| `XENONET_PROJECTS_MAX_DIRS` | `2000`, the most folders one scan reads |
| `XENONET_STATE_DIR` | `%LOCALAPPDATA%\XenoNet`, or `~/Library/Application Support/XenoNet` on macOS |
| `XENONET_CACHE_DIR` | `%LOCALAPPDATA%\XenoNet\Cache`, or `~/Library/Caches/XenoNet` on macOS |
| `XENONET_CAPTURES_DIR` | Posters go in `Pictures\XenoNet`, and clips in `Videos\XenoNet`. On macOS, clips go in `Movies/XenoNet`. |
| `XENONET_WIKI_DIR` | The shared context's folder. XenoNet finds it from Xenolith's settings, or Agent Wiki's when Xenolith isn't installed ([Shared context](shared-context.md)). Otherwise there's none. Any folder with `pages/*.md` works, and it can also have `log/<year>/<date>.md` and `inbox/*.md`. |
| `XENONET_CLAUDE_USAGE` | On. Set it to `off` to stop asking Anthropic for Claude plan usage. |
| `XENONET_ACTIVE_MINUTES` | `60`, how recently a session must have run in a project for the map to light the project |
| `XENONET_CODEX_BIN` | The `codex` bundled with the ChatGPT desktop app, or else `codex` on your `PATH`. The bundled one is the newest `codex.exe` on Windows, and `ChatGPT.app/Contents/Resources/codex` on macOS. |
| `XENONET_CLAUDE_DESKTOP_DIR` | XenoNet finds it by itself. It's `%APPDATA%\Claude` or its MSIX copy on Windows, and `~/Library/Application Support/Claude` on macOS. |
| `XENONET_HIDE_RESOURCES` | MCP servers that belong to the host apps, which don't get their own node |
| `XENONET_IDENTITIES` | On. Set it to `off` to stop reading the apps' sign-ins, gh's accounts and each repo's git identity. |
| `XENONET_HIDE_IDENTITIES` | Off. Set it to `1` to start with identities masked. |
| `XENONET_ACTORS` | On. Set it to `off` to turn [actors](actors.md) off. Then there's no layer and no observers, and nothing is reported, you included. |
| `XENONET_NUMBAT_FILE` | Unset. Name the NDJSON file numbat writes events to, and XenoNet shows the runs in it as sessions, observed through numbat. See [Related work](related-work.md#numbat-runs-xenonet-doesnt-read-itself-experimental). |
| `XENONET_DEMO` | Set it to `1` for demo mode, the same as `npm run demo`. |
| `XENONET_UPSTREAM` | XenoNet Dev only: another XenoNet's address on this computer, such as a forwarded port, whose data the page shows, read-only. See [Developing on another computer](development.md#developing-on-another-computer). |
| `XENONET_ASSISTANT` | On. Set it to `off` to leave out [remote actors' connections](cloud-assistants.md) altogether: no pairing, no private endpoint. |
| `XENONET_GATEWAY_PORT` | The service's port plus 1010, so `48850`. A gateway you already set up keeps the port it had. See [the actor gateway](actors.md#actors-outside-this-machine-the-gateway). |
