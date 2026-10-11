# Sessions working together

Several Claude Code, Codex and Grok Build sessions often work in the same repo at once, sometimes in the same working tree. Without XenoNet, they can't see each other. So two of them edit the same file, or one rebuilds what another just changed.

XenoNet's engine starts from what XenoNet already reads in every transcript: who is working where, on which branch, and editing which files. It adds messages between sessions, claims on files, and a line from each session about what it's doing. On the map, each message travels as a light from one session to the other.

## Connecting your agents

`Connect your agents to XenoNet` in the palette shows what it will add to Claude Code's, Codex's and Grok Build's settings. Grok Build shows there once it's installed. When you confirm, it adds the `xenonet` MCP server for every project, and hooks. You do this once.

From then on:

- When a session starts, it hears which other sessions have worked in its repo in the last half hour, worktrees included, and what each is doing.
- Before a session edits a file another session is working on, XenoNet warns it. A file counts as another session's when that session edited it in the last half hour in the same working tree, changed it in another worktree, or claimed it. A change in another worktree is a merge conflict to come. The warning doesn't stop the edit.
- A session gets messages from other sessions between its tool calls, or before it finishes. XenoNet marks each message as coming from the session or actor that sent it, not from you.
- Each session can use the tools `peers`, `who_has`, `send`, `inbox`, `claim`, `release`, `focus` and `doctor`. `send` reaches one session, everyone in the repo, or everyone. `inbox` can wait for a reply. `doctor` says what's wrong with XenoNet's setup.

The tools know which session is calling without asking it. Claude Code tells its MCP servers the session's ID, and Codex sends the thread's ID with each call. For Grok Build, XenoNet asks Grok to put the session's ID in the server's environment. Where Grok doesn't, XenoNet finds the session from the grok process the call comes from, which Grok's hooks named.

A message to an idle session waits for its next turn, and the console shows it.

Each message comes with how XenoNet knows who sent it. XenoNet writes this from its own checks when the message arrives, apart from what the sender wrote. The session reads it as a `verified:` line above the message, with the message's own lines quoted beneath it, so nothing in a message can pass for a line of XenoNet's. The inbox has the same as structured data in `envelope`. It holds the sender, how the sender proved who it is (its session's process, an actor's token, or XenoNet's own window), what XenoNet saw of the program that sent it, and how strong that proof is (strong, partial or weak). On Windows and Linux, XenoNet can't yet check which program sent a request, and the line says so. An actor's message then rests on its token alone. A `verified:` line says how XenoNet knows who sent a message. It doesn't say that the message is true, and it approves nothing.

Sessions that are already running pick up the connection when they restart. Codex runs new hooks only once you trust them with `/hooks`. Grok Build runs the hooks in `~/.grok/hooks` without asking.

On Windows, command hooks run through PowerShell. Codex's hook definitions include a Windows command override, and Claude Code's session-start hook uses its PowerShell shell. The remaining Claude Code hooks use HTTP.

`Disconnect`, in the same panel, takes it all out again.

## Grok Build

Grok Build reads a hook's answer only at some events, so a Grok Build session hears XenoNet at different moments:

- It hears which other sessions work in its repo with its first tool result, since Grok ignores what a hook says when a session starts.
- An edit warning arrives with the edit's result, after the edit.
- Messages arrive with tool results and at the end of a turn. Grok drops what a hook says when you send a prompt.
- A subagent's hooks get nothing, and leave the session's messages unread for the session itself.

XenoNet writes its MCP servers into `~/.grok/config.toml` itself, as `[mcp_servers.xenonet]` tables, and leaves the rest of the file as it was, comments included. `grok mcp add` rewrites the whole file and drops its comments, so XenoNet doesn't use it. Its hooks go in a file of their own, `~/.grok/hooks/xenonet.json`. `GROK_HOME` moves both.

Grok Build also runs Claude Code's hooks, from `~/.claude/settings.json`. XenoNet's Claude Code hooks pass over Grok's sessions, since Grok's own speak for them.

## From a terminal

`xenonet connect` and `xenonet disconnect` do the same as the panel. `xenonet agents`, `xenonet who <path>` and `xenonet messages` show what the engine sees. [The xenonet command](cli.md) lists them all.

## Optional Claude Code hooks

XenoNet already catches questions and plan approvals by reading transcripts. Claude Code's hooks also tell it about permission prompts and notifications, which transcripts don't record. [Connecting your agents](#connecting-your-agents) adds these hooks, along with the engine's own.

To add them by hand instead, run `npm run hook:snippet` in a checkout. It prints a settings block for `~/.claude/settings.json`, and it doesn't install anything.
