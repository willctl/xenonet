# XenoNet user guide

XenoNet is the network for SI (super intelligence) on your machine. It has a background service, a CLI, MCP tools and a desktop console. Start with the [front page](../../README.md), and come here for the detail behind it. Many steps here use the palette, the list of commands that `Ctrl+K` opens, or `⌘K` on a Mac.

- [Install](install.md) says how to build XenoNet's installers, and how to install and uninstall it on Windows and macOS.
- [Components and updates](components.md) explains separate installs, the MCP servers and which processes keep running during an update.
- [Keeping the service running](always-on.md) says how to have the service start at login, and start again if it stops, on macOS, Linux and Windows.
- [Finding projects](projects.md) says how XenoNet turns your projects folder into projects and groups, and how to read another folder.
- [Identities](identities.md) says how XenoNet links each session and project to your Claude, ChatGPT, GitHub and git accounts.
- [Sessions working together](agents.md) says how to connect Claude Code, Codex and Grok Build to XenoNet, so each session can see and message the others on your machine.
- [Live delivery and scopes](live-delivery.md) says how messages reach sessions as they're sent, how to choose who gets them with selectors such as `project:atlas app:opencode`, what actors may see, and the guards against misuse.
- [OpenCode sessions messaging each other](opencode-messaging.md) has a demo of OpenCode sessions messaging each other by title and by selector, with its measured times, and says how to set it up in your own projects.
- [Actors](actors.md) covers you at the console, always-on agents and your own programs, which XenoNet shows on a layer of their own.
- [Remote actors](cloud-assistants.md) covers pairing an agent in the cloud, such as an OpenAI dot, so it can read session statuses here without changing anything, and stopping it.
- [A network across your machines](network.md) covers joining the machines you use, so their sessions see and message each other and you can act from any of them: starting a network, adding and removing machines, the gateway, every machine's load in the Machines panel, and what crosses between machines.
- [Other machines over SSH](other-machines.md) covers the prototype that shows other machines' XenoNets beside your own, read-only, without a network.
- [Shared context](shared-context.md) covers Xenolith, the pages every session shares, and how they show on the map.
- [Themes](themes.md) lists the themes that come with XenoNet, and says how to choose one or make your own.
- [The xenonet command](cli.md) lists what the `xenonet` command does in a terminal.
- [When something goes wrong](troubleshooting.md) covers XenoNet's error log, its doctor, and asking Codex or Claude Code about them.
- [Provider refusals](refusals.md) covers the refusals XenoNet keeps, and exporting them for a dataset.
- [Decision models](decisions.md) covers asking a decision model on OpenRouter some of what XenoNet asks Codex.
- [Spotify](spotify.md) covers using XenoNet as a remote for Spotify, and its volume dip while you're needed.
- [Privacy and security](privacy.md) says what XenoNet keeps, what it sends, and how it checks who is asking.
- [How it works](how-it-works.md) lists where XenoNet's data comes from, where XenoNet keeps its files, and the environment variables it reads.
- [Development](development.md) covers working on XenoNet itself, driving it from agents, scripts and tests, and the layout of the code.
- [Provider drift](provider-drift.md) says how the daily check of new Claude Code, Agent SDK and Codex releases works, and how to act on its report.
- [Related work](related-work.md) compares XenoNet with numbat and SPACE, and covers using them with XenoNet, with a table of the sessions and actors XenoNet detects today.
