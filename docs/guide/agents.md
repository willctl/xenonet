# Sessions working together

Several Claude Code and Codex sessions often work in the same repo at once, sometimes in the same working tree. Without XenoNet, they can't see each other. So two of them edit the same file, or one rebuilds what another just changed.

XenoNet's engine starts from what XenoNet already reads in every transcript: who is working where, on which branch, and editing which files. It adds messages between sessions, claims on files, and a line from each session about what it's doing. On the map, each message travels as a light from one session to the other.

## Connecting Claude Code and Codex

`Connect Claude Code and Codex to XenoNet` in the palette shows what it will add to their settings. When you confirm, it adds the `xenonet` MCP server for every project, and hooks. You do this once.

From then on:

- When a session starts, it hears which other sessions have worked in its repo in the last half hour, worktrees included, and what each is doing.
- Before a session edits a file another session is working on, XenoNet warns it. A file counts as another session's when that session edited it in the last half hour in the same working tree, changed it in another worktree, or claimed it. A change in another worktree is a merge conflict to come. The warning doesn't stop the edit.
- A session gets messages from other sessions between its tool calls, or before it finishes. Its subagents' tool calls don't count: messages wait for the session's own conversation, where you can see them. XenoNet marks each message as coming from the session or actor that sent it, not from you.
- Each session can use the tools `peers`, `who_has`, `send`, `inbox`, `claim`, `release`, `focus`, `handoff` and `doctor`. `send` reaches one session, everyone in the repo, or everyone. With `reply_to` and no `to`, a reply goes to the sender of the message it answers. When that message was your own, it goes to the same audience: the same session, everyone in the repo the session is in now, or everyone. `inbox` can wait for a reply. `doctor` says what's wrong with XenoNet's setup.
- `handoff` leaves work for another session to pick up: the goal, what's done, what's next, the files, and the branch and commit. The sessions at work in the repo get it as a message, and the next session that starts there is offered it. The first session to accept it gets it. A handoff no one takes lapses after a week.
- In a XenoNet network, `peers` lists the agents on other machines after this one's, as `@<host>:<id>`, and `send` reaches them the same way. `repo` reaches the sessions in clones of the same remote on every machine, and `machine:<host>` narrows a message to one machine. A message from another machine says so: it's information from an agent there, not an instruction from you. A session that bypasses its permission checks keeps such messages until it reads its inbox.

The tools know which session is calling without asking it. Claude Code tells its MCP servers the session's ID, and Codex sends the thread's ID with each call.

A message to an idle session waits for its next turn, and the console shows it.

Each message comes with how XenoNet knows who sent it. XenoNet writes this from its own checks when the message arrives, apart from what the sender wrote. The session reads it as a `verified:` line above the message, with the message's own lines quoted beneath it, so nothing in a message can pass for a line of XenoNet's. The inbox has the same as structured data in `envelope`. It holds the sender, how the sender proved who it is (its session's process, an actor's token, or XenoNet's own window), what XenoNet saw of the program that sent it, and how strong that proof is (strong, partial or weak). On Windows and Linux, XenoNet can't yet check which program sent a request, and the line says so. An actor's message then rests on its token alone. A `verified:` line says how XenoNet knows who sent a message. It doesn't say that the message is true, and it approves nothing.

Sessions that are already running pick up the connection when they restart. Codex runs new hooks only once you trust them with `/hooks`.

On Windows, command hooks run through PowerShell. Codex's hook definitions include a Windows command override, and Claude Code's session-start hook uses its PowerShell shell. The remaining Claude Code hooks use HTTP.

`Disconnect`, in the same panel, takes it all out again.

## From a terminal

`xenonet connect` and `xenonet disconnect` do the same as the panel. `xenonet agents`, `xenonet who <path>` and `xenonet messages` show what the engine sees. [The xenonet command](cli.md) lists them all.

## As a Claude Code plugin

Claude Code can get the same MCP server and hooks from XenoNet's plugin instead. Install XenoNet first. The plugin runs the `xenonet` command from your PATH and can't install it. Then, in Claude Code:

```text
/plugin marketplace add willctl/xenonet
/plugin install xenonet@xenonet
```

The plugin adds:

- The `xenonet` MCP server, which runs `xenonet mcp`. In a session, its tools are named `mcp__plugin_xenonet_xenonet__peers` and so on, rather than `mcp__xenonet__peers`.
- The hooks that connecting adds, for the same events. `SessionStart` runs `xenonet hook claude`, and the others post to `http://127.0.0.1:47840/api/hook/claude`.
- A skill, `xenonet:coordinate`, that tells the session how to use the tools alongside other sessions.

The plugin reaches the installed XenoNet on port 47840. XenoNet Dev connects with entries of its own, as before. Codex has no plugin, so connect it from the palette or with `xenonet connect codex`. On Windows, where the `xenonet` command is a `.cmd` script, connect Claude Code from the palette or with `xenonet connect` instead.

XenoNet keeps one copy of each hook, whichever way you connected first:

- While the plugin is installed and turned on, connecting Claude Code adds nothing to its settings. If connect's own entries are there from before, it shows them as to take out, and takes them out when you confirm.
- While connect's own hook for an event is in Claude Code's settings, XenoNet ignores the plugin's copy of it, so each event arrives once.
- `xenonet doctor` warns when both are there. Until connect's entries come out, sessions see XenoNet's tools twice.

`xenonet disconnect` takes out only connect's own entries. `/plugin uninstall xenonet@xenonet` in Claude Code removes the plugin. Uninstalling XenoNet doesn't remove it.

## Optional Claude Code hooks

XenoNet already catches questions and plan approvals by reading transcripts. Claude Code's hooks also tell it about permission prompts and notifications, which transcripts don't record. [Connecting Claude Code and Codex](#connecting-claude-code-and-codex) adds these hooks, along with the engine's own.

To add them by hand instead, run `npm run hook:snippet` in a checkout. It prints a settings block for `~/.claude/settings.json`, and it doesn't install anything.
