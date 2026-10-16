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

## What XenoNet detects today

Sessions are runs of a harness; actors last across runs.

| Harness or actor | Kind | How XenoNet knows it | Edits in warnings | Messages |
|---|---|---|---|---|
| Claude Code (CLI, and its sessions in Claude desktop) | Sessions | Reads its transcripts; hooks once you connect it | Yes | Yes, once connected |
| Codex (CLI and app) | Sessions | Reads its rollouts and app-server; hooks once you connect it | Yes | Yes, once connected |
| Runs numbat records (experimental) | Sessions, observed | numbat's event file, when you name it | Yes, observed | No |
| A run in a SPACE sandbox (experimental) | Session, under its actor | `report_run` through the actor gateway | No | No |
| Pi | Runs in a records panel, not the map | Reads its session files, when you turn that on | No | No |
| Hermes Agent | Actor | Observes its processes and state database | Yes, observed | Once it registers |
| Grok Bot (its Mac app) | Actor | Observes the processes its app runs | No | Once it registers or connects through the gateway |
| Muse (its Mac app) | Actor | Observes whether its app runs | No | Through the gateway |
| OpenAI dots | Actor | A remote connection you pair | No | No, it only reads |
| Your own programs | Actor | Register with a token, or connect through the gateway | Yes, when they report them | Yes |
