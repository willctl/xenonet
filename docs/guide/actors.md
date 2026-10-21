# Actors

An actor works across sessions and across days. It can be you at the console, an always-on agent, an agent in the cloud that works through an app on this Mac, or a program of your own.

Actors hover on a layer of their own above the map, with beams into the projects and sessions they act on. The Actors panel lists them, and `Actors` in the palette opens it. What actors do goes in the activity feed, beside what sessions do.

Sessions can see actors too, in `peers` and in their briefings. An actor's recent edits count in the warnings before an edit, and its messages arrive marked as an actor's. An actor's message carries no more weight than any other message, and it can't approve anything.

## On the map

Each actor takes its theme's form. Actors breathe, glow as brightly as they're present, and react when they act, with light running down the beam to what they touched.

An actor waiting on you gets a card in the strip at the top, and a row in the Needs you panel (`N`), which lists everything waiting on you, the longest wait first. A click takes whatever is drawn in front, actor or node. Selecting an actor frames it with what it acts on, and shows it in the inspector. The Actors panel also lists the programs asking to be actors.

## How XenoNet knows an actor

XenoNet knows an actor in one or more of these ways:

| How | What it means |
| --- | --- |
| Observed | XenoNet reads the actor's state on this Mac, for metadata only. It reads which of the actor's processes run, and what its own files say. |
| Attributed | XenoNet traces work here back to the actor by process ancestry. A Claude Code or Codex session that started under one of the actor's processes is the actor's work. |
| Registered | The actor reports itself, with a token that works once you've allowed it. |
| Remote | The actor reads what's going on here through a connection you paired, and nothing more. See [Remote actors](cloud-assistants.md). |
| Demo | The actor is made up, in demo mode. The demo's actors are Wren, Kestrel and Moth. |

## Products XenoNet observes

XenoNet observes three products. It installs nothing in them and changes none of their settings. Their observers are on by default. Turn one off in the Actors panel, or with `xenonet actor observe <product> off`. `XENONET_ACTORS=off` turns actors off altogether.

An actor that XenoNet only observes doesn't report itself, so you can't remove it. Turning its observer off takes it off the map instead. Nothing can message it either, since nothing would read what it's sent. Grok Bot and Muse can also report themselves, so they can message and be messaged: see [Letting Grok Bot and Muse talk](#letting-grok-bot-and-muse-talk).

A product that's installed stays listed, however long it hasn't run.

### Hermes Agent

Hermes Agent, from Nous Research, is `actor:hermes`, and `actor:hermes-<profile>` for each named profile.

- Hermes is working while one of its processes runs and a session was active in the last two minutes. The processes that count are the desktop backend, the gateway, the CLI and the terminal UI.
- It's idle while one of those processes runs and no session was active.
- It's away when its gateway's heartbeat goes quiet.

XenoNet reads Hermes's spawn ledger, gateway state, scheduler ticker, scheduled jobs and background processes. From Hermes's session database, XenoNet reads which sessions are active and where, which tools ran when, and the path of each file Hermes writes. SQLite takes that path out of the call, so nothing else in the call leaves the database.

Those edits count in the warnings a session gets before an edit. XenoNet marks them as observed, since it inferred them.

Hermes's status is in XenoNet's words, such as "Running a command in atlas-api". The sessions it starts with `claude -p` or `codex exec` get a beam from it.

### Grok Bot

Grok Bot, from SpaceXAI, is `actor:grok-bot`.

- It's idle while its app runs.
- It's working while a command the app runs is running.
- With the app closed, it's out of sight, never offline, since Bots carry on in the cloud. It stays listed that way for as long as the app is installed.

While the app runs, XenoNet looks at what it runs every 3 seconds. Each command counts in the project it ran in, by place and count, never by its arguments. A command that starts `claude` or `codex` counts as starting a session.

XenoNet reads processes. Of the app's files, it reads only whether the app is there, and two keys of its `Info.plist`: `CFBundleShortVersionString` for its version, and `CFBundleIdentifier`, so that another app named Grok Bot isn't taken for Cursor's.

### Muse

Muse, Meta's personal agent, is `actor:muse`.

- It's idle while its Mac app runs.
- With the app closed, it's out of sight, never offline, since Muse works in Meta's cloud whether or not the app is open. It stays listed that way for as long as the app is installed.
- It's never shown as working. What Muse does on your Mac, such as reading files or using other apps, can't be traced to it without reading protected system data, so XenoNet doesn't try.

XenoNet reads processes. Of the app's files, it reads only whether the app is there, and two keys of its `Info.plist`: `CFBundleShortVersionString` for its version, and `CFBundleIdentifier`, so that another app called Muse isn't taken for Meta's. It never reads Muse's data, caches, web storage, preferences or sign-in.

Muse can't be paired as a remote actor yet: its custom connectors take an API key or OAuth for a web service, and don't connect to MCP servers. It can report itself, from a program it runs on your Mac (below).

### Letting Grok Bot and Muse talk

Grok Bot and Muse both work in their makers' clouds, but each can run commands on your Mac through its app: Grok Bot with **Execution on Local Computer**, Muse with its **Filesystem** connector, which runs programs. Keep both on **Ask every time**, so you approve each command. With that, each can register as its own actor, joining the one XenoNet observes, and then message the other through XenoNet:

1. Ask it to run this on your Mac:

   ```console
   $ xenonet actor register --product grok-bot --name "Grok Bot" --capabilities message-agents --heartbeat 3600 --token-file ~/.config/xenonet/actors/grok-bot.token
   ```

   For Muse, use `--product muse --name Muse` and its own file, `muse.token`.
2. Allow it in XenoNet's window. The card says which app asked.
3. From then on, it sends with `XENONET_ACTOR_TOKEN_FILE=~/.config/xenonet/actors/grok-bot.token xenonet actor send actor:muse "…"`, and reads what it's sent with `xenonet actor inbox` and the same variable.

Each token works only from a command run under its own app, as its maker signed it. That's Cursor for Grok Bot and Meta for Muse. XenoNet refuses it from your terminal, from the other actor, from the app's own processes, and from a copy of the app that someone else built. Each message reaches the other with XenoNet's note of who sent it, as messages between sessions do.

Once one reports itself, XenoNet leaves what it did to its own reports and keeps its presence: Grok Bot's commands no longer count as `ran` by themselves. Removing it in the Actors panel puts it back as XenoNet observes it.

Muse can't run commands on your Mac yet, so it takes the gateway instead (below). Grok Bot can take either.

### Actors outside this machine: the gateway

The actor gateway is an MCP server for actors that work in their makers' clouds, such as Muse or a Grok Bot. Through it they can message the sessions and actors here, and read their own messages, with the tools `send`, `inbox`, `peers`, `whoami` and `heartbeat`. With `report_run`, an actor that drives a harness somewhere else, such as Claude Code in a sandbox, shows each run as a session under it (see [Related work](related-work.md#space-a-run-in-a-sandbox-shown-as-a-session-experimental)). They can reach nothing else of XenoNet through it.

It listens on this computer alone, on XenoNet's port plus 1010 (48850), or the port `XENONET_GATEWAY_PORT` names. Once you set it up, it keeps that port, even if XenoNet's own port changes. A tunnel puts it on the internet. XenoNet doesn't assume you have any particular one: you pick it, and XenoNet runs it with the sign-in that tool already has.

XenoNet used to put the gateway on its port plus 10 (47850), which is Xenolith's port. A gateway you set up then keeps that port, so its tunnel and the apps that use it go on working: XenoNet saves the port with the gateway's settings the first time it starts. To move it off Xenolith's port, start XenoNet with `XENONET_GATEWAY_PORT` set to the port you want, then point the tunnel there: XenoNet sets up again the tunnels it runs itself, and the window offers Repair for Tailscale Funnel.

| Tunnel | What you need | Its address |
|---|---|---|
| Tailscale Funnel | Tailscale, signed in, with Funnel allowed for this computer in your tailnet's policy | This computer's name on your tailnet. Funnel's certificate makes that name public, so rename the computer in Tailscale first if its name says more than you'd like. |
| Cloudflare Tunnel | `cloudflared`, and a tunnel you made and routed to a name on your domain (`cloudflared tunnel create xenonet`, then `cloudflared tunnel route dns xenonet <the name>`) | The name you routed |
| Cloudflare quick tunnel | `cloudflared` alone, no account | A new one every time it starts, so it's for trying the gateway out |
| ngrok | `ngrok`, with your authtoken added | Your ngrok domain, if you give it |
| Your own | Anything that forwards `https://<your name>/mcp` to the gateway's port and passes the Host header on | The name you give |

To set it up:

1. In the Actors panel, under **actor gateway**, choose **Set up…** and pick a tunnel. The ones this computer has come first. XenoNet starts the tunnel, then checks that the address reaches the gateway from the internet side, and shows **Online** once it does.
2. On the actor, choose **Connect…**. XenoNet makes a gateway token, which works only at the gateway. For Muse, which XenoNet only observes until then, this also makes it an actor that reports itself.
   A session can start this step for you, in Claude Code or Codex with the `ask_gateway_token` tool, or from a terminal with `xenonet gateway token <slug>`. That makes no token and returns none. It leaves a card in XenoNet's window, in the attention strip and the Actors panel, saying whose app it's for and who asked. **Allow** makes the token and opens the steps below, with the token masked. **Deny** drops the ask. Until you allow, the actor's old token keeps working, and an ask lapses after ten minutes. The session never sees the token, so don't paste one to it or into a chat.
3. Follow the numbered steps XenoNet shows, each with its own Copy button. The token stays masked until you choose **Show**, and for ten seconds after, and **Copy token** copies it without showing it. The last step waits for the app's first call and ticks when it comes:
   - **Grok Bot**: copy the message into a new Grok Bot chat. It adds XenoNet as an MCP server by URL with the token in its header. The token stays in that chat, so delete the chat afterwards.
   - **Muse**: copy the message into a Muse chat and approve the connector card. When Muse asks for the key, paste the token into the card's key field, not the chat.
   - **Any other MCP host**: add an MCP server by URL, with the gateway's address and the header `Authorization: Bearer <the token>`.

Once an app calls, its row in the panel says when it last did.

#### When something changes

- **After a restart or reboot**, XenoNet starts the gateway and its tunnel again as you left them. Tailscale keeps Funnel set up by itself; XenoNet checks it's still there and sets it up again if not. Installed XenoNet starts at login, so the gateway is back without opening the window.
- **While it's on**, XenoNet checks the address every ten minutes, every two while the check fails, and right away when the computer wakes. A tunnel XenoNet runs that exits is started again. The panel and `xenonet doctor` say what's wrong in words, and **Repair** sets the tunnel up again.
- **If the address changes**, because you picked another tunnel, renamed the computer, or a quick tunnel restarted, each app's token still works, and the app needs only the new address. The row of each app that last called the old one says so, and **Send the new one** shows what to tell it.
- **A gateway token unused for 30 days** stops working. The panel warns you a week before. If an app calls with a token that was replaced or taken away, its row says so, and **Reconnect** gives it a new one.

When a message through the gateway reaches a session or actor here, XenoNet marks it as sent with a gateway token from outside this machine. That proof is weaker than a process XenoNet checked here. Like any message, it approves nothing. `xenonet gateway` shows the same from your own terminal, `xenonet gateway tunnels` lists the tunnels and which ones this computer has, and `xenonet gateway off` closes the gateway and stops the tunnel XenoNet runs.

### Waking an actor when a message arrives

An actor doesn't need to keep checking its inbox. XenoNet can tell it as soon as a message arrives, in one of three ways:

- **Listening.** A program of your own that stays running hears of each message at once with `xenonet actor listen`, under its own token. With `--exec <program>`, XenoNet runs that program for each message and passes the message as JSON on its standard input. The program doesn't get the actor's token. Treat the message as untrusted input, since any session can write it.
- **A webhook.** XenoNet calls a URL you give it, which starts the actor, and the actor then reads its inbox. Choose **Wake…** on the actor, then **Webhook**. For Grok Bot, XenoNet gives you a message that asks it to make a routine that starts from a webhook. You then copy the routine's Webhook URL and Webhook key into XenoNet. **Test** calls it once so you can see it work.
- **Notify me.** For an actor nothing can wake, such as Muse, XenoNet notifies you once a message has waited two minutes, at most once an hour per actor. This is the default for an actor connected through the gateway.

A wake says only that messages are waiting, and how many. It never includes what they say, who sent them, or a token. The actor reads them with its own token.

Any session can write to an actor, and each wake may start a run you pay for, so XenoNet limits wakes:
- Messages that arrive close together make one wake.
- An actor is woken at most every 30 seconds, and 20 times an hour.
- An actor isn't woken again while the messages from its last wake are still unread, for up to ten minutes.
- A webhook that fails is retried after a longer wait each time, up to half an hour. The actor's row and `xenonet doctor` say why it failed.

A webhook's URL and key go to your computer's credential store: Keychain on a Mac, Credential Manager on Windows, the Secret Service on Linux. The window never shows them again. The store keeps them out of XenoNet's files, but programs running as you can still read them there. So before each call, XenoNet checks the URL again and calls it only if its host is still the one you set. XenoNet calls `https://` addresses only, or `http://` on this computer. It refuses link-local addresses and doesn't follow redirects. The key is sent as `Authorization: Bearer`, which is what a Grok Bot routine expects. A key that starts with `whsec_` signs the call instead, following [Standard Webhooks](https://www.standardwebhooks.com). Only XenoNet's window can set a webhook. `xenonet actor wake <slug>` shows how an actor is woken, `test` calls its webhook, and `off` stops waking it.

## Remote actors

An agent in the cloud, such as an OpenAI dot, can read session statuses and counts here through a connection you pair in the Actors panel. It reads only the projects you pick, changes nothing, and loses access when its grant expires. It shows beside the map like any actor, and nothing can message it. A remote connection can also join an actor XenoNet already knows, such as Grok Bot or a Hermes profile, without touching its token or what XenoNet observes of it. [Remote actors](cloud-assistants.md) has the rest.

## You

You are `actor:you`, and XenoNet never names you. Only XenoNet's window reports you. It reports what you look at, read, open, start, hand off or open a terminal in, as node ids and actions, and never anything you type.

Everywhere else, you're a placeholder: in the list other programs read, in the event stream, and to sessions. So nothing can time its work to your absence.

While XenoNet's window is hidden, you're away, and ten minutes later you're offline. XenoNet keeps your record for 24 hours. Turning off `Show me on the map` in the Actors panel deletes it at once, and the window reports nothing more of you.

## What each actor's work costs

Under its estimate for today, Token use lists what each actor's work would cost at API prices. The three that spent the most show first, the rest are under **more**, and what no actor is tied to comes last. Hover over a row to see how many sessions it covers. `xenonet spend` says the same from a terminal.

Each session counts for one actor at most, so nothing is counted twice and the rows add up to the day's estimate. A session is an actor's when XenoNet knows the actor started it:

1. A run an actor reported through the [gateway](#actors-outside-this-machine-the-gateway) is that actor's.
2. Otherwise, a session an actor started is its own, whether XenoNet traced it back to the actor's processes or the actor reported it. When two actors say they started the same session, what XenoNet traced itself comes first, then the earliest.
3. Otherwise, a subagent, or a session another session started, counts for the actor of the session it came from. A fork doesn't, since whoever forked a session may not be whoever started it.
4. Otherwise, the session is unattributed.

The sessions you start yourself are unattributed too. The window tells XenoNet you started a session in a project, not which session, and XenoNet doesn't guess from timing. An actor that isn't in the Actors panel, such as one whose observer you turned off, gets no sessions either.

A run reported from elsewhere has no transcript on this computer, so XenoNet can't price it. Its actor's row says how many such runs it had today, and they aren't in its cost.

## Your own programs

Your own programs register with the `xenonet` command, and wait for your Allow in XenoNet's window. Every new actor waits for it, even one you register from your own terminal.

The request shows as a card in the strip at the top, in the Needs you panel and in the Actors panel, with what it asks for and which program asked. Allow makes its token work. Deny, or ten minutes without an answer, revokes the token. `Add an actor` in the Actors panel does it all in one step.

```console
$ xenonet actor register --name Wren --capabilities run-commands,edit-repos,message-agents --token-file ~/.config/wren/xenonet.token
Waiting for you to allow "Wren" in XenoNet's window (it asks for 10 minutes)…
✓ actor:wren is allowed. Its token is in ~/.config/wren/xenonet.token, readable only by you.

$ export XENONET_ACTOR_TOKEN_FILE=~/.config/wren/xenonet.token
$ xenonet actor heartbeat --presence working --status "Rebuilding the docs site"
$ xenonet actor event ran --path ~/Projects/docs-site --count 2
$ xenonet actor send repo "Docs site rebuilt; don't touch dist/ for a minute"
```

An actor reports facts, such as what kind of thing it did, where, and how many times. XenoNet writes every word the map shows and sessions read, except the actor's own one-line status.

An actor is held to the capabilities you allowed, and to limits on how much and how often it reports.

With `message-agents`, an actor can write to sessions and to other actors of yours (`xenonet actor send actor:moth "…"`). Each reads it in its inbox, with how XenoNet knows who sent it. To write to an actor yourself, choose **Message…** in its inspector. The actor reads your message in its inbox as from you, at XenoNet's own window. Only you can write and send there. XenoNet's control channel never presses those buttons outside the demo, so no agent driving the window can send as you.

A program that speaks MCP gets the same calls as tools from the `xenonet` MCP server, with `XENONET_ACTOR_TOKEN_FILE` in its environment. The tools are `actor_heartbeat`, `actor_events` and `actor_targets`. The [actor interface](../design/actors/interface.md) has the whole API.

## Tokens

A token is `xenonet_actor_` followed by 64 hex digits. XenoNet shows it once, and keeps only a hash of it. The window masks it as `xenonet_actor_••••`, and leaves it out of the page until you choose **Show**, which shows it for ten seconds. **Copy** copies it without showing it. XenoNet masks a shown token again when you switch to another app, while identities are hidden, and in screenshots an agent takes of the window. The token travels in the `Authorization: Bearer` header, from a file only you can read, never as an argument or in a URL.

The `xenonet` command won't write a token into a git working tree or the projects folder unless you add `--force`. The doctor looks for tokens left in the repos it knows, ignored files such as a `.env` included. It looks once a day, and whenever you run it yourself.

On macOS, a token is also held to processes. It never works from under Claude Code or Codex, or from an actor app's own process. A product's token works only from under that product.

`New token` in the Actors panel replaces a token. The panel says whether each token works, when it expires, and when it was last used. A token unused for 30 days stops working, and the doctor says so.
