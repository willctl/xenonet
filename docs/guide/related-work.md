# Related work

Other open-source projects work on agents from different sides. XenoNet can use two of them.

- [numbat](https://github.com/perplexityai/numbat), from Perplexity, audits what coding agents do, for security. It hooks desktop, CLI and IDE agents and writes their actions as NDJSON events with published JSON Schemas.
- [SPACE](https://github.com/perplexityai/SPACE), from Perplexity, isolates where agents run. It starts Firecracker microVM sandboxes through an HTTP and server-sent events API.
- XenoNet coordinates the sessions that are live on your machine. It shows them on a map, warns a session before it edits a file another one is changing, and lets them message each other.

Both are Apache-2.0. XenoNet includes none of their code. It reads numbat's documented output format and calls SPACE's documented API.

In XenoNet, a **session** is the state around one run of a harness: a Claude Code session, a Codex thread, a Gemini CLI run. An **actor** is something that lasts across runs: you, Hermes Agent, Grok Bot, Muse, or a program of your own. Sessions sit on the map in their projects. Actors hover on a layer of their own, with beams to the sessions and projects they act on.

## numbat: runs XenoNet doesn't read itself (experimental)

XenoNet reads Claude Code's and Codex's sessions itself. With numbat, the runs of the other agents numbat records show as sessions too: one per numbat session id, with the agent as its app, such as `gemini-cli:<id>`. Each is marked as observed through numbat. Its edits count in the warnings another session gets before it edits the same file, marked as observed, since XenoNet inferred them from records rather than a transcript. An observed session has no hooks here, so it can't be sent messages, and it has no transcript to preview.

It's off unless you turn it on:

1. Have numbat's hooks write events to a file, for example `numbat hook install --agent gemini --emit events --output file --output-file ~/.numbat/records.ndjson`. numbat's [live capture guide](https://github.com/perplexityai/numbat/blob/main/docs/live-capture.md) covers each agent.
2. Start XenoNet with `XENONET_NUMBAT_FILE` set to that file.

XenoNet reads the file and never writes to it. From each event it keeps the agent, the session id and its parent's, the kind of event, the edited file's path, the project folder, the branch, the model and the time. It never keeps the endpoint (host and user names), commands, prompts, previews, or tool input and output. Claude Code and Codex events are left out, since XenoNet reads those runs itself, and so are events that name no session. XenoNet reads numbat event schemas 0.2.0 to 0.5.0 and passes over other records, such as findings. A run drops off a day after its last event, and `session.end` ends it.

This is tested with made-up events that fit numbat's published schema, not yet against every agent numbat covers.

## SPACE: a run in a sandbox, shown as a session (experimental)

A harness running in a SPACE sandbox can show in XenoNet as a session, through XenoNet's [actor gateway](actors.md#actors-outside-this-machine-the-gateway). The gateway token belongs to an actor, the identity that lasts. Each sandbox reports its run as a session under that actor, so one actor can drive many sandboxes, and each run is its own session. The gateway takes a gateway token made in XenoNet's window, listens on this machine alone, and reaches the internet only through a tunnel you pick. It allows messages, presence and run reports, and nothing else: no files, no session contents, no control.

1. Turn the gateway on with a tunnel, in the Actors panel or with `xenonet gateway on`.
2. In the Actors panel, choose **Add** for a program of your own, such as "SPACE runner", then **Connect…** on it to make its gateway token.
3. Put `scripts/space/presence.mjs` in the sandbox's template, for example at `/opt/xenonet/presence.mjs`. It needs Node 18 or later and nothing else.
4. Start it in the sandbox with the gateway's address, the token, the run's name and its harness in the command's own `env`. Never put the token in the sandbox's `metadata`, its name or the template. From the machine that runs SPACE:

   ```bash
   SPACE_API_URL=… SPACE_API_TOKEN=… \
   XENONET_GATEWAY_URL=https://<your gateway address>/mcp XENONET_GATEWAY_TOKEN_FILE=<file with the token> \
   node scripts/space/presence.mjs space-command sbx_… claude
   ```

   That sends `POST /v1/sandboxes/{id}/commands` with `{"command": ["node", "/opt/xenonet/presence.mjs"], "env": {…}}`, naming the run after the sandbox.

In the sandbox, the script calls `whoami`, then every minute sends the actor's `heartbeat` and `report_run` for its run, with the line in `XENONET_STATUS` as the session's title. Stopped, it reports the run ended and leaves the actor as it was, since other runs may go on. It takes only `https://` addresses, or `http://` on loopback, and follows no redirects. The sandbox needs outbound access to the gateway's address.

The run shows in the Elsewhere zone, since its folder isn't on this machine, with a beam from its actor. It's marked as reported through the gateway, can't be sent messages, and has no transcript here. XenoNet keeps runs in memory: one goes an hour after its last report, or ten minutes after it ended. A gateway token unused for 30 days stops working, and **Remove** in the Actors panel takes it away at once.

This is tested against XenoNet's real gateway and a stand-in SPACE API. It isn't yet tested on a Firecracker host.

## Native readers: other harnesses' own records (experimental)

XenoNet can read some other harnesses' sessions from their own records, without numbat. Each reader is off unless `XENONET_READERS` names it, as a comma-separated list. A session such a reader finds is read, not observed: its edits count in warnings like any session's. XenoNet has no hooks into these harnesses, so their sessions can't be sent messages yet, and have no transcript to preview.

**OpenCode** (`XENONET_READERS=opencode`). Each OpenCode session is a session, `opencode:<id>` with a dash for each underscore in OpenCode's id (`ses_4f…` is `opencode:ses-4f…`), with a subagent's run under the session that started it. XenoNet opens OpenCode's database, `opencode.db`, read only for each look: `OPENCODE_DB`, or `$XDG_DATA_HOME/opencode/`, or `~/.local/share/opencode/` on every OS. It reads each session's id, parent, folder, title, version and times, and the files its `edit`, `write`, `multiedit` and `apply_patch` tools changed. It never reads prompts, replies, tool output or file contents. A session drops off a day after its last change. When numbat records the same run, it's one session, read here. This is tested with a made-up database and against sessions OpenCode 1.19 wrote.

**Gemini CLI** (`XENONET_READERS=gemini-cli`). Each Gemini CLI session is a session, `gemini-cli:<session id>`, with a subagent's run under the session that started it. XenoNet reads its chat records, `~/.gemini/tmp/<project>/chats/session-*.jsonl` (or under `GEMINI_CLI_HOME`), and the project's folder from `.project_root` beside them. It reads each session's id, times and summary, and the files its `write_file` and `replace` calls changed. It never reads prompts, replies, thoughts or tool results. When numbat records the same run, it's one session, read here. This is tested with made-up records in the shape Gemini CLI 0.64's source writes, not yet against a real session.

**GitHub Copilot CLI** (`XENONET_READERS=copilot`). Each Copilot CLI session is a session, `copilot:<session id>`. XenoNet reads its event log, `~/.copilot/session-state/<id>/events.jsonl` (or `<id>.jsonl`, the layout before 1.0.11): the session's id, working folder, branch, title and times, the file each edit tool names, and the files its shutdown says it changed. It never reads prompts, replies, commands or tool results. A session ends at its shutdown. This is tested with made-up events in the shape of the Copilot SDK's published event schema, not yet against a real session.

**Cursor** (`XENONET_READERS=cursor`). Cursor's agent sessions are read through Cursor's [hooks](https://cursor.com/docs/agent/hooks), since Cursor documents no session format. Each conversation is a session, `cursor:<conversation id>`, in its first workspace folder. Add XenoNet's hook to `~/.cursor/hooks.json` for `sessionStart`, `afterFileEdit` and `sessionEnd`, telling it the file to write. The hook is a script in a checkout of XenoNet's source, `scripts/cursor/hook.mjs`; the installed app doesn't include it yet.

```json
{
  "version": 1,
  "hooks": {
    "sessionStart": [{ "command": "node /path/to/xenonet/scripts/cursor/hook.mjs /path/to/cursor-hooks.ndjson" }],
    "afterFileEdit": [{ "command": "node /path/to/xenonet/scripts/cursor/hook.mjs /path/to/cursor-hooks.ndjson" }],
    "sessionEnd": [{ "command": "node /path/to/xenonet/scripts/cursor/hook.mjs /path/to/cursor-hooks.ndjson" }]
  }
}
```

Then start XenoNet with `XENONET_CURSOR_EVENTS` set to that file (by default it reads `cursor-hooks.ndjson` in XenoNet's state folder). From each hook's input the hook keeps the conversation id, the Cursor version, the workspace folders, the edited file's path and how the session ended. It never keeps the prompt, an edit's text, a file's content, the model or your email. It always answers `{}` and never blocks the agent. A session ends at `sessionEnd`. This is tested with made-up hook input in the shape of Cursor's hooks reference, not yet in Cursor itself.

## What XenoNet detects today

Sessions are runs of a harness; actors last across runs.

| Harness or actor | Kind | How XenoNet knows it | Edits in warnings | Messages |
|---|---|---|---|---|
| Claude Code (CLI, and its sessions in Claude desktop) | Sessions | Reads its transcripts; hooks once you connect it | Yes | Yes, once connected |
| Codex (CLI and app) | Sessions | Reads its rollouts and app-server; hooks once you connect it | Yes | Yes, once connected |
| Gemini CLI (experimental) | Sessions | Reads its chat records, when you turn that on | Yes | No |
| Cursor (experimental) | Sessions | XenoNet's hook in Cursor's hooks.json, when you turn that on | Yes | No |
| GitHub Copilot CLI (experimental) | Sessions | Reads its event logs, when you turn that on | Yes | No |
| OpenCode (experimental) | Sessions | Reads its database, when you turn that on | Yes | No |
| Runs numbat records (experimental) | Sessions, observed | numbat's event file, when you name it | Yes, observed | No |
| A run in a SPACE sandbox (experimental) | Session, under its actor | `report_run` through the actor gateway | No | No |
| Pi | Runs in a records panel, not the map | Reads its session files, when you turn that on | No | No |
| Hermes Agent | Actor | Observes its processes and state database | Yes, observed | Once it registers |
| Grok Bot (its Mac app) | Actor | Observes the processes its app runs | No | Once it registers or connects through the gateway |
| Muse (its Mac app) | Actor | Observes whether its app runs | No | Through the gateway |
| OpenAI dots | Actor | A remote connection you pair | No | No, it only reads |
| Your own programs | Actor | Register with a token, or connect through the gateway | Yes, when they report them | Yes |
