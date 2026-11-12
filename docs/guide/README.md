# XenoNet user guide

XenoNet helps actors, people and bots, take an intent to an outcome together. The work runs on hosts (your machines), as identities (your accounts), in running sessions (Claude Code, Codex, Grok Build and other agents), and every actor works within the grant it holds. Each actor gets the same kind of grant, to read and to act, scoped to the machines, projects and sessions you allow, and issuing or widening one takes you at the machine. [How it works](how-it-works.md) says how the pieces fit.

XenoNet is a background service on each machine, the `xenonet` command, MCP tools for agents and a desktop console. Start with the [front page](../../README.md), and come here for the detail behind it. Many steps here use the palette, the list of commands that `Ctrl+K` opens, or `⌘K` on a Mac.

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
- [A network across your machines](network.md) covers joining the machines you use, so their sessions see and message each other and you can act from any of them: starting a network, adding and removing machines, relays and mailboxes, sharing projects with each network, every machine's load in the Machines panel, and what crosses between machines.
- [Running your own relay](own-relay.md) covers what a relay for your machines needs, its config file and service, adding it to a network or to one machine, and trusting a relay with a certificate from your own CA.
- [Delegating work to another machine](delegation.md) covers setting a machine to take delegated work once, handing it a task from an assistant, what runs without a question, and what always comes to you.
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
- [Related work](related-work.md) compares XenoNet with numbat and SPACE, and covers using them with XenoNet, with a table of the sessions and actors XenoNet detects today.
