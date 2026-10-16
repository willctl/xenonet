# Related work

Other open-source projects work on agents from different sides. XenoNet can use two of them.

- [numbat](https://github.com/perplexityai/numbat), from Perplexity, audits what coding agents do, for security. It hooks desktop, CLI and IDE agents and writes their actions as NDJSON events with published JSON Schemas.
- [SPACE](https://github.com/perplexityai/SPACE), from Perplexity, isolates where agents run. It starts Firecracker microVM sandboxes through an HTTP and server-sent events API.
- XenoNet coordinates the sessions that are live on your machine. It shows them on a map, warns a session before it edits a file another one is changing, and lets them message each other.

Both are Apache-2.0. XenoNet includes none of their code. It reads numbat's documented output format and calls SPACE's documented API.

## numbat: agents XenoNet doesn't read itself (experimental)

XenoNet reads Claude Code and Codex itself. With numbat, the other agents numbat records can show in XenoNet too. Each agent becomes an observed [actor](actors.md), such as `actor:numbat-gemini-cli`, shown as "Gemini CLI (through numbat)". Its edits count in the warnings a session gets before it edits the same file, marked as observed, since XenoNet inferred them.

It's off unless you turn it on:

1. Have numbat's hooks write events to a file, for example `numbat hook install --agent gemini --emit events --output file --output-file ~/.numbat/records.ndjson`. numbat's [live capture guide](https://github.com/perplexityai/numbat/blob/main/docs/live-capture.md) covers each agent.
2. Start XenoNet with `XENONET_NUMBAT_FILE` set to that file.

XenoNet reads the file and never writes to it. From each event it keeps the agent, the kind of event, the edited file's path, the project folder, the session id and the time. It never keeps the endpoint (host and user names), commands, prompts, previews, or tool input and output. An edit outside every place on the map is left out. Claude Code, Codex and Hermes events are left out too, since XenoNet already reads those itself. XenoNet reads numbat event schemas 0.2.0 to 0.5.0 and passes over other records, such as findings.

`xenonet actor observe numbat off` stops reading it, and `on` starts again.

This is tested with made-up events that fit numbat's published schema, not yet against every agent numbat covers.

## SPACE: a sandbox that shows in XenoNet (experimental)

An agent running in a SPACE sandbox can show in XenoNet as an actor, with a status line you see, through XenoNet's [actor gateway](actors.md#actors-outside-this-machine-the-gateway). The gateway takes a gateway token made in XenoNet's window, listens on this machine alone, and reaches the internet only through a tunnel you pick. It allows messages and presence and nothing else: no files, no session contents, no control.

1. Turn the gateway on with a tunnel, in the Actors panel or with `xenonet gateway on`.
2. In the Actors panel, choose **Add** for a program of your own, such as "SPACE runner", then **Connect…** on it to make its gateway token.
3. Put `scripts/space/presence.mjs` in the sandbox's template, for example at `/opt/xenonet/presence.mjs`. It needs Node 18 or later and nothing else.
4. Start it in the sandbox with the gateway's address and token in the command's own `env`. Never put the token in the sandbox's `metadata`, its name or the template. From the machine that runs SPACE:

   ```bash
   SPACE_API_URL=… SPACE_API_TOKEN=… \
   XENONET_GATEWAY_URL=https://<your gateway address>/mcp XENONET_GATEWAY_TOKEN_FILE=<file with the token> \
   node scripts/space/presence.mjs space-command sbx_…
   ```

   That sends `POST /v1/sandboxes/{id}/commands` with `{"command": ["node", "/opt/xenonet/presence.mjs"], "env": {…}}`.

In the sandbox, the script calls `whoami`, then `heartbeat` every minute with the line in `XENONET_STATUS`, and says it's offline when it's stopped. It takes only `https://` addresses, or `http://` on loopback, and follows no redirects. The sandbox needs outbound access to the gateway's address.

The sandbox shows as an actor, not as a Claude Code or Codex session. The gateway token is the actor's, so sandboxes that share it show as one actor. To tell them apart, add one actor per sandbox. A gateway token unused for 30 days stops working, and **Remove** in the Actors panel takes it away at once.

This is tested against XenoNet's real gateway and a stand-in SPACE API. It isn't yet tested on a Firecracker host.

## What XenoNet detects today

| Agent or source | How XenoNet knows it | On the map | Edits in warnings | Messages |
|---|---|---|---|---|
| Claude Code (CLI, and its sessions in Claude desktop) | Reads its transcripts; hooks once you connect it | Sessions | Yes | Yes, once connected |
| Codex (CLI and app) | Reads its rollouts and app-server; hooks once you connect it | Sessions | Yes | Yes, once connected |
| Hermes Agent | Observes its processes and state database | Actor | Yes, observed | Once it registers |
| Grok Bot (its Mac app) | Observes the processes its app runs | Actor | No | Once it registers or connects through the gateway |
| Muse (its Mac app) | Observes whether its app runs | Actor | No | Through the gateway |
| OpenAI dots | A remote connection you pair | Actor | No | No, it only reads |
| Pi | Reads its session files, when you turn that on | A records panel, not the map | No | No |
| Your own programs | Register with a token, or connect through the gateway | Actor | Yes, when they report them | Yes |
| Agents numbat records (experimental) | numbat's event file, when you name it | Actor per agent | Yes, observed | No |
| A SPACE sandbox (experimental) | The actor gateway | Actor | No | Yes, through the gateway |
