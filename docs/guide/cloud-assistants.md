# Remote actors

SI (super intelligence) that runs somewhere else, such as an OpenAI dot in ChatGPT's cloud, can read session statuses on this computer through a connection you pair. XenoNet calls it a remote actor. It can see which sessions are working, waiting, stuck or finished, and how much they used. It can't change anything.

This is new and experimental. It has been tested on Windows with a local MCP client and with made-up data. It hasn't been tested with a real dot through OpenAI's tunnel yet, so treat dot support as unconfirmed until you've run the checks under [Trying it first](#trying-it-first).

## What it can read

A connection reads through six tools, and has no others:

| Tool | What it answers |
| --- | --- |
| `xenonet_status` | Who it is to XenoNet, which computer this is, what it may read, and how fresh each source is. |
| `xenonet_projects` | The projects you picked, each with how many sessions are working, waiting, stuck, completed, idle or unknown over the last seven days. |
| `xenonet_sessions` | Sessions in those projects, newest first, with each one's status and what it was read from. |
| `xenonet_attention` | Sessions waiting on their user, sessions that look stuck, two sessions editing the same files, and turns that ended. |
| `xenonet_session` | One session, with its token and tool-call counts if it asks. |
| `xenonet_actors` | Itself, and other actors acting on those projects. |

It sees only the projects you pick. A connection starts with none. Projects you add to the map later are never added to a connection by themselves.

Without labels, it gets ids, statuses, times and counts. With labels turned on, it also gets project names, session titles and actor names, cleaned of anything that looks like a key, an email or a folder named for a person. Titles are text that SI and people wrote, so they can say anything. Turning labels on sends that text.

It never gets transcripts, prompts, messages, a question's text, tool names, arguments or output, file paths, branches, git remotes, accounts, credentials, error messages, or anything from [other machines](other-machines.md). It can't run commands, open terminals, read files, take screenshots, use the console or message SI sessions.

## Pairing one

1. Open the Actors panel from the palette, and choose `Pair a remote actor…` at the bottom.
2. Choose the actor. For a dot, choose dots, then a new one or a dot you paired before. Grok Bot and Hermes Agent can be paired to their existing actors, but their own connections aren't tested yet.
3. Name the connection, and say who can call its endpoint, in your own words.
4. Pick the projects it may read, whether labels go, and how long the grant lasts. It lasts 30 days unless you choose 1, 7, 14, 60 or 90.
5. Choose `Preview what leaves`. XenoNet shows made-up answers first, then the exact fields it would send for the projects you picked.
6. Choose `Approve and pair`. XenoNet shows the command your tunnel client runs.

XenoNet writes the connection's credential to its state folder, readable only by your account, and never shows it. The command holds no secret:

```bash
xenonet mcp assistant --grant gr_…
```

Until you've checked who can call the endpoint and ticked `I checked who can call this endpoint`, the connection reads made-up data only, and every answer says so.

Pairing happens only in XenoNet's window. A remote actor can't pair itself, widen its own access or approve anything.

## Connecting a dot through OpenAI's tunnel

OpenAI's Secure MCP Tunnel lets ChatGPT, the Responses API and Codex call an MCP server on your computer through an outbound connection, so nothing listens on your network. You create the tunnel on OpenAI's side, following [OpenAI's current instructions](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels): a tunnel on the Tunnels page, a runtime API key restricted to Tunnels (Read and Use), and the plugin in ChatGPT.

### Letting XenoNet run it

XenoNet can run OpenAI's `tunnel-client` for a connection, so the tunnel is up while XenoNet is and you don't keep a terminal open for it:

1. Install `tunnel-client` from [OpenAI's releases](https://github.com/openai/tunnel-client/releases). XenoNet looks on your PATH, in `%LOCALAPPDATA%\Programs\tunnel-client` on Windows, and in Homebrew's folders and `~/.local/bin` elsewhere. `XENONET_TUNNEL_CLIENT` names another place. XenoNet never downloads it.
2. In the Actors panel, choose **Tunnel…** on the connection's row, or **Set up the tunnel…** right after pairing.
3. Give the tunnel's id (`tunnel_…`). If a `tunnel-client` profile of your own already runs this connection, XenoNet finds it and offers its tunnel.
4. Paste the runtime API key and choose **Keep it**. XenoNet keeps it in Windows Credential Manager, the macOS keychain or the Secret Service (`secret-tool`) on Linux, gives it only to `tunnel-client` in its own environment, and never shows it again. If you keep the key there already under a name of your own, give that name under **kept as** instead.
5. Choose **Check** to have `tunnel-client doctor` look at the setup, then **Start**.

The row then says whether the tunnel runs, when it last heard from OpenAI, and what went wrong if it stopped. XenoNet starts it again after a few seconds if it exits, and gives up after three tries in ten minutes. **Start it when XenoNet starts** is off unless you turn it on. **Stop**, revoking the connection, the connection expiring, or turning cloud access off stops it at once.

XenoNet writes the tunnel's profile in its own state folder, with no key in it, and runs XenoNet's launcher as the MCP server, so updates to XenoNet don't change the command.

### Running it yourself

You can also run `tunnel-client` yourself and give it XenoNet's command as a stdio MCP server, for example:

```bash
tunnel-client init --sample sample_mcp_stdio_local --profile xenonet --tunnel-id <your tunnel> --mcp-command "xenonet mcp assistant --grant gr_…"
tunnel-client run --profile xenonet
```

The tunnel's API key then stays in the tunnel client's own configuration. If a profile you run yourself has a fixed health address, XenoNet sees that it's running and won't start a second client for the same connection.

### Who can call it

A tunnel follows your OpenAI organization and ChatGPT workspace, so people in them may be able to call it, depending on your workspace's settings. Check that before you tick `I checked who can call this endpoint`, and say who can call it when you pair. The tunnel doesn't tell XenoNet which ChatGPT user or which dot is calling, so a dot's account stays unverified in XenoNet, or your word if you link it to a ChatGPT identity XenoNet knows.

## Trying it first

`xenonet mcp assistant --fixture` serves the same six tools over made-up data, with no grant and nothing of this computer's. Point an MCP client or the tunnel at it first, and check:

1. The client lists the six tools and calls `xenonet_status`.
2. A second `xenonet_sessions` call a few minutes later shows different statuses with new times.
3. A call with an argument the tool doesn't take is refused.

Then pair a connection without ticking the verified box, which still reads made-up data, and check the same through the real route. Only then pair one for real data, with one project and labels off, and compare what it says with what XenoNet shows.

## Stopping it

- `Revoke` on a connection's row stops it at once, for good, and deletes its credential. Its actor, its history and its other connections stay.
- `Cloud access` beside Remote connections turns every connection off at once. Turning it on again happens only in XenoNet's window.
- From a terminal, `xenonet assistant revoke <grant>` and `xenonet assistant disable` do the same, and work with XenoNet stopped. They leave a tombstone in XenoNet's state folder that every version of XenoNet honors, so restarting, updating or restoring an older copy of XenoNet's database can't bring a connection back.
- Removing the actor in the Actors panel revokes all of its connections.

Revoking doesn't take back what a remote actor already read. Revoke the tunnel or the endpoint with its provider too.

`xenonet assistant` lists the connections, with when each was last used, and `xenonet assistant audit` lists what was read lately: times, operations, result codes and sizes, never content. XenoNet keeps that log for seven days.

## On the map

A remote actor appears beside the map like any actor, marked as reading through a connection you paired. Each read it makes counts as being at work for 90 seconds; after that it's out of sight until it reads again. A read of one project or session draws a beam from it, as watching.

An actor that reaches XenoNet through a tunnel, such as a dot through OpenAI's Secure MCP Tunnel, has a gate below it: a ring with hex digits round its rim and a conduit up to the actor, and its beams go out through the gate. The gate is open while an adapter is connected through the tunnel, and shut, a dim ring, while none is. Each read sends light down the conduit and through the gate to the projects and sessions it read; a read of made-up data reaches nothing on the map. The inspector says which route it takes. Its connection being up says nothing about whether the actor itself is working, and its connection being down doesn't mean the actor stopped.

A remote actor can't receive messages, so SI sees it in `peers` as unreachable.

## Limits

- 60 reads a minute for each connection, in bursts of up to ten, and four at a time.
- Lists come 20 at a time, and at most 50. Page tokens last five minutes.
- Sessions can be listed up to 30 days back, seven by default.
- Eight remote actors and 16 active connections.
- An answer says `live` only when its sources were read in the last 30 seconds. When XenoNet can't be reached, the adapter can answer from what it read in the last 15 minutes, marked `stale`, and only while the connection still stands with the same projects and labels.

If XenoNet's state folder turns up on another computer or under another account, every connection stops and has to be paired again.
