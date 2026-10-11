# Privacy and security

## The server and the page

- The server listens on `127.0.0.1` only. It rejects other `Host` headers, which blocks DNS rebinding, and it rejects cross-origin POST requests.
- The page runs under a Content-Security-Policy. Its scripts come only from XenoNet's own server, and nothing inline or evaluated runs. It connects only to that server, and no other page can frame it. So a slip in escaping what it shows from transcripts, actors or other machines can't run script.
- The terminal runs in the desktop app's main process, and talks to the page over IPC. The web server never exposes it.
- Every URL XenoNet opens is built from IDs it already knows. A handoff brief is the only text that reaches a URL, as an encoded query parameter.

## What leaves this computer

- Installed copies check GitHub for a release 30 seconds after startup and every six hours. On Windows, XenoNet downloads the installer and waits for you to choose when to restart. GitHub receives ordinary connection details, such as your IP address, but no session titles, transcripts, project paths or credentials. Turn off automatic checks in Updates to stop these requests. Manual checks still contact GitHub. Source and demo copies never check.

- XenoNet reads Claude Code's sign-in for each usage request, and uses it only for that request to Anthropic. XenoNet never stores, logs or refreshes it. The keychain item also holds other apps' tokens, and XenoNet takes only the Claude token from it. An expired token waits for Claude Code to refresh it. `XENONET_CLAUDE_USAGE=off` stops the request.
- With an OpenRouter key you gave it, XenoNet can send that same outline to a [decision model](decisions.md) instead of Codex, when you choose Send to jev. It asks OpenRouter for zero data retention and sends no user or session ids. With that key, XenoNet also asks OpenRouter what the key has spent, at most once a minute while a window is open; that request carries the key and nothing else.
- Asking Codex which folders are projects sends only an outline of folder names under `~/Projects`, and the names of well-known files. It never sends the path above `~/Projects`, so it never sends your user name. It sends nothing until XenoNet has shown you all of it. See [Asking Codex which folders are projects](projects.md#asking-codex-which-folders-are-projects).
- When you turn on [other machines](other-machines.md), XenoNet reads them over SSH, with its own ssh config, and checks host keys strictly. Nothing listens on a network, and XenoNet keeps no keys. A host sends only titles, states and counts, with its home folder as `~`, and XenoNet only reads from it. XenoNet checks and caps what a host sends, and a host's ids can't pass for this machine's or another host's.

- A [remote actor](cloud-assistants.md) you pair reads, through the tunnel you set up, the statuses, times and counts of sessions in the projects you picked, and their names and titles only if you turn labels on. It never reads transcripts, prompts, messages, paths, branches, accounts or credentials, and it changes nothing. Pairing shows exactly what would leave before anything is approved. Revoking stops it at once but doesn't take back what it already read. XenoNet keeps a log of each read, without content, for seven days. If you let XenoNet run the tunnel, it runs OpenAI's `tunnel-client` while XenoNet runs, which keeps an outbound connection to OpenAI open, and keeps the tunnel's runtime key in your computer's credential store. The key goes only to `tunnel-client`, never into a file, a log or the window.

- The [actor gateway](actors.md#actors-outside-this-machine-the-gateway) is off until you turn it on. Then it listens on this computer alone, and the tunnel you pick puts it on the internet. XenoNet runs that tool (Tailscale, cloudflared or ngrok) with the sign-in it already has, and holds none of its secrets. While the gateway is on, XenoNet checks the address every ten minutes. It asks public DNS resolvers (Cloudflare's, Google's and Quad9's) for the name and calls the address once. Tunnels with certificates for your name, Tailscale Funnel's among them, put that name in public certificate logs. The gateway answers only its own address, only at `/mcp`, and only to gateway tokens you made in XenoNet's window. Through it, an actor in its maker's cloud can see who is at work here (titles, apps, statuses, projects and what each says it's doing), send messages to the sessions and actors here, and read the messages sent to it. It can't read files, transcripts, prompts or settings, and can't change anything else. XenoNet marks the messages that come through it as gateway messages. Whoever holds a gateway token can do what that actor can, so keep each token in its own app's settings. Connect… makes a new one, and the old one stops working at once.

- [Waking an actor](actors.md#waking-an-actor-when-a-message-arrives) when a message arrives sends nothing unless you set it up. A webhook you give XenoNet gets one call per wake, at most every 30 seconds, saying which actor it's for and how many messages wait. It never includes what they say, who sent them, or a token. Its URL and key stay in your computer's credential store. A notification for an actor nothing can wake says the same, and only on this computer.

- XenoNet's [error log](troubleshooting.md) stays in its state folder, with home folders as `~` and anything like a key taken out. It never holds transcripts, prompts or titles. `xenonet diagnose` and Copy diagnostics make a summary for you to share, and send nothing themselves. Ask Codex or Claude Code starts a session on this computer, which talks to its provider as any session of yours does. That session reads the summary and XenoNet's code, and its instructions tell it never to file or send anything.

- [Provider refusals](refusals.md) XenoNet notices are kept in its state folder until you delete them: the refusal itself always, and the transcript up to it only if you turn keeping context on. They never leave this computer; `xenonet refusals export` writes them to a file for you, scrubbed of home folders and keys with `--scrub`.

- If you [connect Spotify](spotify.md), XenoNet talks to Spotify to sign in, to read what's playing every few seconds while a window is open, and to send the commands you give or turn on. Nothing of your sessions goes to Spotify, and what's playing goes nowhere else.

## What shows on screen

- Paths under your home folder show as `~` on the map, in the panels and in the activity feed. So a screenshot of them doesn't show your user name.
- Identities stay on this computer, and nothing about them goes into a prompt for Codex. Only their colors reach the map, so posters, replay clips and wallpaper mode never show an email or a handle.
- To mask identities in the panels too, for screenshots and screen sharing, use `Hide or show identities` in the palette, `◐ hide` under Filters, `?private` in the page's address, or `XENONET_HIDE_IDENTITIES=1`. Masked identities show as "GitHub A" or "Claude B".
- Demo mode's identities are made up, and its git reads only the demo's own config.

## Repo status

XenoNet runs `git status` every 20 seconds for each repo's status. So that a repo you cloned or unpacked can't make XenoNet run its programs, XenoNet runs it this way:

- It empties the filter drivers that the repo's own config names. The filters of git-lfs stay on. Your own filters, from your global and system config, work as they do anywhere.
- It points the repo's hooks at nothing.
- It turns the repo's fsmonitor off.

As a result, a file that an emptied filter keeps different in the repo can count as modified. A submodule counts as changed only when it's on another commit.

## Sessions and actors

- Messages between sessions, their claims and what each says it's doing stay on this computer, in `engine.sqlite` in the state folder, for a week.
- XenoNet changes Claude Code's and Codex's settings only when you confirm in `Connect Claude Code and Codex to XenoNet`. It shows each change first, and adds only its own entries. The first time, it keeps a copy of each settings file, named `<file>.before-xenonet`. `Disconnect` takes it all out again.
- XenoNet marks a message from another session as that session's, never as yours, and the message can't approve anything.
- Actors stay on this computer too, in the same `engine.sqlite`. Their events stay for a week. Yours stay for a day, and only XenoNet's window sees them.
- Actors' tokens are kept only as hashes. The doctor looks for tokens left in repos, ignored files such as a `.env` included, once a day and whenever you run it yourself.
- XenoNet's observers read other apps' state for metadata only. They never read message content, prompts, titles, secrets or process environments. Of Hermes's tool calls, XenoNet reads which tools ran when and the path of each file one wrote, and nothing else of a message. Of Grok Bot, XenoNet reads only its processes, and the two keys of its app's `Info.plist` that hold its version and bundle id. Of Muse, XenoNet reads only whether its app runs, and the two keys of its `Info.plist` that hold its version and bundle id. See [Actors](actors.md).
- Before a newer XenoNet changes the shape of `engine.sqlite`, it copies the file beside it, as `engine.v<n>.sqlite`. It deletes the copy a week later, so the copy doesn't keep for good what the file keeps for a week or a day.

## Telling sessions apart on macOS

On macOS, one session can't pass for another. Any program here can reach the server and give any session's name, but it can't hide which process it is. XenoNet looks up the process on the other end of each connection, and that process's parents. A process is under a session when that session's process is one of its parents.

- A request in a Claude Code session's name has to come from that session's process, or from under it, such as from its MCP server, its hooks or its shell. A request in a Codex thread's name has to come from that thread's process, or from under it.
- Claude Code's hooks come from Claude Code itself. So when that process is signed by Anthropic, its hooks settle which process holds a session. A program merely named `claude` doesn't count.
- XenoNet refuses a request whose process it can't see, such as one from a program that sends it and exits at once.
- XenoNet refuses to connect Claude Code and Codex when the request comes from under any session.
- XenoNet also refuses changes to your standing settings from under a session or an [actor product](actors.md). These settings are the projects folder, your rules for it, asking Codex about it and Codex's picks, identities' names, hidden flags and pins, and the theme. You change them from XenoNet's window or your own terminal.

This check can't tell apart threads of one Codex process. It also can't place a session that no running process has been seen to hold yet, so that session goes to whoever names it first.

On Windows and Linux, XenoNet doesn't check requests yet.
