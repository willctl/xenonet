# Delegating work to another machine

When you tell an assistant on one machine "do this on the other one", the work should start there and get done without you walking over to it. Delegation is how XenoNet allows that: you set a machine to take delegated work once, from your own console, and from then on an assistant hands it a task, the task runs, and the result comes back to the assistant that asked.

## Setting it, once

On the machine that will do the work, set its **start** setting to **Delegated**: in the console, open the Network panel (the palette's Network settings), find the machine's row, and press **Delegated** under **Start sessions**. On this machine's own settings the buttons are in the same place.

This widens what the network may do there, so it asks you (Touch ID on a Mac, Windows Hello or polkit elsewhere), and it's done in XenoNet's window: `xenonet net set --start delegate` in a terminal refuses and says so. Putting another machine on delegate takes the founder, or waits for a second window host to approve it, the same as letting the network start sessions there. Stepping back to **The network**, **You** or **Nobody** is done at once, from any window host or your own terminal.

The first time, it also issues your **delegation**: an entry in your identity, signed by your key, that lets your machines start sessions and answer their routine prompts for work you started on any of them. It names all your machines' keys, those you add later too, and lasts a year. Issuing it asks you twice: once to approve it, once to sign it. A machine needs a key of yours to be named in it, so a machine that joined without your identity can't take delegated work, and XenoNet says so when you press it.

The machine that does the work checks both: its own start setting, and a delegation of yours that names both its key and the key of the machine the task came from. Since the standard delegation names every machine of yours, a session on any of them can hand work to any other you set to Delegated: one machine, or each of several. The console shows who set it and when under the machine's settings: "Delegated by inix, Nov 1."

## What an assistant does

An assistant calls the xenonet MCP server's `delegate` tool with the machine, a task, and optionally a folder there:

```text
delegate machine: neo-rex, task: "Update XenoNet to the latest release, then report the version and what the doctor says."
```

The answer comes at once. When the new session stops, the assistant is told what it came to (finished, stopped to ask something, was denied a tool, failed, or left nothing), with its last words, and is woken if it is idle. The session needn't be told to report: XenoNet does it.

A machine that isn't set to Delegated refuses the task, and says how you set it. The assistant isn't started as a plain agent start instead: `delegate` is for machines you decided to delegate to, and `start_session` is the tool for a machine that lets the network start sessions without a delegation.

To hand a task to an agent outside your machines that speaks A2A, the assistant calls `delegate` with `actor` instead: see [Handing work to an A2A agent](actors.md#handing-work-to-an-a2a-agent). Intents, where you make the request at your window and name the machine that works it, are next on the plan ([direction.md](../design/direction.md)).

## What the machine tells the new session

The first prompt of a delegated session says whose delegation it runs on, which the machine checked:

> [XenoNet] inix says this was asked by the session «Fix the login form» (claude:…). Your user delegated it: they put this machine on standing delegation at inix's console on 2026-11-01, which XenoNet here checked; the asker can't claim it.

The machine writes that line from its own settings and its copy of your identity. What the asker sent is the task, after the line, and can't change it. A start from an agent without a delegation says "it came from an agent, not from your user", so a careful session stops to ask before anything that matters; under a delegation it doesn't have to ask about what the delegation covers.

## What it covers

A delegated session is a Claude Code run, and its permission prompts come to XenoNet, as any run's do. XenoNet answers them itself when they are within the delegation, and shows the rest in **Needs you**, on every window host, where you allow or deny them from any of your machines. An Allow there asks you once (Touch ID on a Mac); a Deny asks nothing. An assistant can't answer for you.

Covered, with no question:

- reading files, searching and listing, except files that look like secrets (`.ssh`, `.aws`, `.env`, anything named for credentials, tokens or passwords);
- creating and editing files under the folder the session started in;
- routine commands: listing, printing and searching, read-only git, and XenoNet's own `--version`, `doctor`, `status`, `logs`, `update` and `service`, one after another with `;`, `&&`, `||` or `|`;
- the xenonet tools that report back (`send`, `inbox`, `peers`, `who_has`, `whoami`, `doctor`, `focus`, `claim`, `release`).

Always a question, whatever the task says:

- deleting or formatting anything, stopping processes, changing the machine;
- secrets, tokens, passwords and credentials, in a path or a command;
- the network: `curl`, `wget`, the web tools, a push;
- writing through a redirect, and a command inside a command;
- any file outside the session's folder, and any command or tool not listed above, including starting another session.

The question says why it wasn't covered: "[outside what you delegated: it has rm in it] rm -rf build". Nobody answering in ten minutes is a deny. XenoNet keeps which sessions run under a delegation in its store for a week, so their prompts are judged against it after the service restarts too.

## Limits and logs

The limits above are fixed in this version; the folder is the one the session starts in, `~` by default. A delegated start counts against the machine's starts an hour like any other, and is refused while XenoNet is paused there. Each prompt the delegation allows is in the machine's log (`xenonet logs`), under `ask`, with the tool and what it asked to do.
