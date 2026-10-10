# Agents working together

Several agents often work in the same repo at once, sometimes in the same working tree. Without help, they can't see each other. So two of them edit the same file, or one rebuilds what another just changed.

XenoNet's engine lets them work together. It starts from what XenoNet already sees in every transcript, which is who is working where, on which branch, and editing which files. It adds messages between agents, claims on files, and a line from each agent about what it's doing. On the map, messages fly between the agents.

## Connecting your agents

`Connect your agents to XenoNet` in the palette shows what it will add to Claude Code's and Codex's settings. When you confirm, it adds the `xenonet` MCP server for every project, and hooks. You do this once.

From then on:

- When a session starts, it hears which other agents have worked in its repo in the last half hour, worktrees included, and what each is doing.
- Before a session edits a file another agent is working on, XenoNet warns it. A file counts as another agent's when that agent edited it in the last half hour in the same working tree, changed it in another worktree, or claimed it. A change in another worktree is a merge conflict to come. The warning doesn't stop the edit.
- A session gets messages from other agents between its tool calls, or before it finishes. Each message is marked as coming from an agent, not from you.
- A session has the tools `peers`, `who_has`, `send`, `inbox`, `claim`, `release`, `focus` and `doctor`. `send` reaches one agent, everyone in the repo, or everyone. `inbox` can wait for a reply. `doctor` is for when something's wrong with XenoNet.

The tools know which agent is calling without asking it. Claude Code tells its MCP servers the session's ID, and Codex sends the thread's ID with each call.

A message to an idle agent waits for its next turn, and the console shows it.

Each message comes with how XenoNet knows who sent it, written from XenoNet's own checks when it arrived and kept apart from what the sender said. An agent reads it as a `verified:` line above the message, whose own lines are quoted beneath it, so nothing in a message can pass for a line of XenoNet's. The inbox has the same as structure (`envelope`): the sender, how it proved who it is (its session's process, an actor's token, or XenoNet's own window), what XenoNet saw of the program that sent it, and how far that goes (strong, partial or weak). On Windows and Linux, XenoNet can't yet check which program sent a request, and the line says so: an actor's message is then its token alone. A `verified:` line says how XenoNet knows who sent a message, not that what it says is true, and it approves nothing.

Sessions that are already running pick up the connection when they restart. Codex runs new hooks only once you trust them with `/hooks`.

On Windows, command hooks run through PowerShell. Codex's hook definitions include a Windows command override, and Claude Code's session-start hook uses its PowerShell shell. The remaining Claude Code hooks use HTTP.

`Disconnect`, in the same panel, takes it all out again.

## From a terminal

`xenonet connect` and `xenonet disconnect` do the same as the panel. `xenonet agents`, `xenonet who <path>` and `xenonet messages` show what the engine sees. [The xenonet command](cli.md) lists them all.

## Optional Claude Code hooks

XenoNet already catches questions and plan approvals by reading transcripts. Claude Code's hooks also tell it about permission prompts and notifications, which transcripts don't record. [Connecting your agents](#connecting-your-agents) adds these hooks, along with the engine's own.

To add them by hand instead, run `npm run hook:snippet` in a checkout. It prints a settings block for `~/.claude/settings.json`, and it doesn't install anything.
