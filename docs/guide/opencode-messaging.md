# OpenCode sessions messaging each other

OpenCode sessions on one machine can message each other by name through XenoNet. One session asks "docs site" a question, and the session with that title gets it in its TUI right away, starts a turn and answers. This page covers a demo that shows it with real OpenCode TUIs, what it measured, and how to set it up in your own projects. [Live delivery and scopes](live-delivery.md) covers how messages get in, for every harness.

## What the demo shows

`scripts/demo/opencode-named.mjs` starts a XenoNet from this checkout and three OpenCode TUIs in tmux. Each TUI works in a project of its own, with a session title of its own and a `NOTES.md` the others ask about:

| Session | Project | Its notes |
|---|---|---|
| Atlas: billing API | atlas | codename Juniper, version 2.4.1 |
| Borealis: docs site | borealis | codename Larch, version 0.9.0 |
| Cirrus: mobile app | cirrus | codename Sorrel, version 5.1.3 |

In each round, the demo puts three questions into the asking sessions' TUIs, as if you had typed them:

1. Atlas asks `docs site` for its release codename. Those are words from Borealis's title.
2. Cirrus asks `app:opencode` which version each is on. That's a selector, so Atlas and Borealis both get it.
3. Borealis asks `project:cirrus app:opencode` for its codename.

The rest is the sessions' own work. XenoNet puts each message into the recipient's TUI through OpenCode's own server, so the recipient starts a turn while it sits idle. It reads its `NOTES.md` and answers with `xenonet_reply`, and the answer comes back the same way. At the end the demo prints what each session said and how long each step took.

## What you need

- macOS or Linux, or WSL on Windows, with Node 22.13 or later and tmux.
- This checkout, with `npm install` run in it.
- OpenCode. The demo was checked with OpenCode 1.18.15. It uses OpenCode's free models, `opencode/big-pickle` unless you pick another, with no account and no key. They need the internet.

## Running it

```sh
node scripts/demo/opencode-named.mjs
```

| Option | What it does |
|---|---|
| `--sessions 2` | Two sessions instead of three: one question by title words, one by selector. |
| `--rounds n` | How many rounds, 1 to 4. The default is 2. XenoNet wakes a session at most 12 times an hour for each sender. |
| `--model opencode/<model>` | Another of OpenCode's models. If OpenCode doesn't have it, the demo lists the free ones it has. |
| `--out <folder>` | Keeps the panes' text, the timings and XenoNet's and OpenCode's records there. |
| `--keep` | Leaves the TUIs and XenoNet running at the end, and prints how to watch a session with `tmux attach` and how to stop it all. |
| `--opencode <path>` | The OpenCode to run, when it isn't on your `PATH` or in `~/.opencode/bin`. |
| `--seed-consent` | Allows the sessions without `xenonet live allow`, for a run under an agent. See Consent, below. |

The demo leaves your own setup alone. Its XenoNet has a port, a state folder and a home folder of its own, so it reads none of your sessions. The TUIs get an empty home and config, so OpenCode uses none of your settings, sign-ins or sessions. tmux runs a server of its own, without your tmux config. All of it lives in one folder under the system's temp folder, which goes at the end with the tmux server and the XenoNet. The first time a project runs, OpenCode installs its plugin package into the project's `.opencode` folder, as it does for any project with one.

The sessions can read files and use XenoNet's tools, and nothing else. Their config turns off editing, the shell, the web and subagents.

### Consent

XenoNet puts a message into an OpenCode session only once you allow that session. The demo runs `xenonet live allow` for each session it starts, which works when you run the demo in your own terminal. XenoNet refuses it from under an agent, such as Claude Code, Codex or OpenCode, since consent is yours to give. Then the demo prints the commands to run in your own terminal and waits for them. `--seed-consent` writes the same allowance into the demo XenoNet's own state and starts it again instead. That XenoNet is always a new one in the demo's folder, so nothing else is changed.

## What it measured

On a Mac with Apple silicon, OpenCode 1.18.15 and `opencode/big-pickle`, three sessions and three rounds made 21 messages and 24 deliveries. Every answer was right, and all 24 deliveries went in through OpenCode's server. Times are from the moment the sender's tool call started:

| Step | Median | Range |
|---|---|---|
| XenoNet takes the message | 61 ms | 10 to 91 ms |
| OpenCode's server takes it | 75 ms | 20 to 98 ms |
| The recipient's turn starts | 82 ms | 21 ms to 1.4 s |
| The answer is back at XenoNet, for the 12 answers | 0.8 s | 0.6 to 11.2 s |

Most of XenoNet's time is reading the process table, to check that the message came from an OpenCode process. It keeps what it read for a second and a half, so a session's next message is taken in about 10 ms. OpenCode's server takes the message some 15 ms later. An idle recipient starts its turn at once. The slow starts, up to 1.4 s, were recipients still finishing a turn of their own. OpenCode holds the message until that turn ends. The rest is the model, reading its notes and answering.

A second run, with two sessions, two rounds and `opencode/deepseek-v4.1-flash-free`, made 8 messages. XenoNet took them in a median of 65 ms and OpenCode's server in 82 ms, the recipient's turn started in 84 ms, and the answers took 2.0 to 7.4 s.

## In your own projects

1. Start each OpenCode TUI with a server XenoNet can reach: `opencode --hostname 127.0.0.1`. Add `--port <n>` to pick the port.
2. Give its sessions XenoNet's tools. Copy `scripts/demo/opencode-tool.js` to `.opencode/tools/xenonet.js` in the project, or to `~/.config/opencode/tools/xenonet.js` for every project. The tools are `xenonet_peers`, `xenonet_send`, `xenonet_reply` and `xenonet_inbox`. They reach the installed XenoNet, on port 47840. To reach another, start OpenCode with `XENONET_PORT` set.
3. Allow each session XenoNet may wake: `xenonet live allow ses_…`, with the id OpenCode shows, or XenoNet's own `opencode:ses-…`. An allowance lasts 8 hours, or what `--hours` says.

Then a session can message another by words from its title, by its id, or by a selector such as `app:opencode` or `project:atlas app:opencode`.

## Good to know

- XenoNet lists the OpenCode sessions you allowed, and those that used its tools, with the titles OpenCode gives them. It doesn't list every OpenCode session it can see.
- The OpenCodes you run read the same database, unless you give them different data folders, so each OpenCode's server knows every session. XenoNet posts to the OpenCode that is running a turn of the session, or else to one started in the session's folder. When no TUI was started there, another OpenCode may run the turn, out of sight of the TUI that shows the session.
- OpenCode's server takes a message as soon as it has stored it. If the model fails after that, XenoNet still counts the message as delivered, and OpenCode's log has the error.
- XenoNet checks that a request in an OpenCode session's name comes from an OpenCode process or from under one, and refuses what's yours to do, such as `xenonet live allow`, from under OpenCode, as it does from under Claude Code. It can't check which OpenCode session sent it, since any OpenCode can open any session.
- OpenCode's free models change. If the demo's default is gone, `--model` takes one of the free ones it lists.
