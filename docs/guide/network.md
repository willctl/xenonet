# A network across your machines

A XenoNet network joins the machines you use, each running XenoNet. The sessions on each machine see the sessions on the others, message them and hear when one of them is changing the same file in its clone. What waits on you on any machine shows in every console, and you can message any session from any machine you use a XenoNet window on.

The machines reach each other straight, over QUIC ([iroh](https://www.iroh.computer)): on your LAN, by direct paths, or through a relay you name. There is no server in the middle. What one machine sends another is signed by it and sealed for the one it's for, so a relay or a mailbox passes it on and can't read it. Only machines in the network can connect.

Each machine keeps working on its own. Its map, messages, claims and warnings never wait on another machine. What it has for a machine that's away goes when that machine is back.

A machine can be in several networks, with an id of its own in each, and each network sees only the projects you share into it.

The design is in [docs/design/network.md](../design/network.md).

## Words

- A **host** is a machine in the network. You pick its id, such as `studio` or `laptop`: lowercase letters, digits and dashes. XenoNet never uses the machine's own name.
- The **founder** is the machine that started the network. Its key is the network's root: if the founder is lost for good, the network is made again.
- A **window host** is a machine you use a XenoNet window on. From its window you act on the other machines: message their sessions as you, and add or remove machines. A server you only reach in a terminal isn't one.
- The **roster** is the signed list of the network's machines, their keys, their settings, its relays and its mailboxes. Every machine keeps a copy and checks each change against it.
- **Your identity** is a key of yours on each machine that can ask you (Touch ID on a Mac, Windows Hello on Windows, polkit or a passphrase on Linux), in a chain every machine keeps a copy of. Whatever XenoNet does as you on another machine is signed by it, and what lets anyone do more (adding a machine, widening a setting, an Allow) asks you first. The design is in [Authorization](../design/authorization.md).

## Starting a network

On the first machine, open **Network settings: your other machines** from the palette (`Ctrl+K`, or `⌘K` on a Mac), choose **Start a network**, and give the network a name and this machine an id. Pick its relay there too: none, your own, or n0's ([Relays](#relays)). That sets up your identity on this machine, which asks you once. Once it's set up, a terminal does the same:

```bash
xenonet net create --name home --host studio
```

This machine is now the founder. Add `--relay https://relay.example.net` to name your own relay, or `--relay n0` for n0's. Adding a relay asks you, so when your terminal can't, the network starts without it and says so; add it from the window then.

A machine already in a network can start another one beside it: **Start another network…** in Network settings, or `xenonet net create` again. Everything that acts on one network then takes `--network <id|name>`, and without it the command asks which.

### The firewall

Other machines connect to this one straight, so its firewall has to let the service accept connections. The firewalls of macOS and Windows let a program in by its path, and the service runs from `runtime/components/service/active/bin/xenonet` in XenoNet's state folder, a path that stays the same across Service updates (see [components](components.md#updates)).

- **On a Mac** with the firewall on, macOS asks once whether `xenonet` may accept incoming connections: choose **Allow**.
- **On Windows**, a service that Task Scheduler starts never gets Windows Defender Firewall's question, so allow it yourself: **Windows Security › Firewall & network protection › Allow an app through firewall › Allow another app**, and pick `active\bin\xenonet.exe` under `runtime\components\service`. Or, in a terminal run as administrator, `netsh advfirewall firewall add rule name="XenoNet Service" dir=in action=allow program="<that path>" enable=yes profile=private`. Until a Service update has made `active`, the service runs from its version's own folder, so allow it once more after that first update.

`xenonet doctor` says whether the firewall lets the service in, and what to run when it doesn't: on a Mac it reads the application firewall's list; on Windows it reads Windows Defender Firewall's profiles and its inbound rules for the service's program, and gives that `netsh` line with the exact path, for the private profile (a network set to Public should be made Private first). It only reads; it never changes a firewall rule.

## Adding a machine

1. On a window host, open the palette and choose **Add a machine**. Give the new machine's id, and tick **I'll use a XenoNet window on it** if you will.

   XenoNet shows one line, masked until you show it:

   ```bash
   xenonet net join xni2_… KX7Q-49MZ-T2
   ```

   The first part is the invite: the network, this machine's key, the relay the network names, if any, and a ticket. It holds no secret and no address. The second part is the code, which is the secret. It works once, for ten minutes, and this machine keeps only its hash. When the network has no relay, the line ends with `--at <address>` (this machine's address) or `--mdns` (find it on the LAN). Only the window makes invites, since signing the machine in asks you: `xenonet net invite` in a terminal says so.

2. On the new machine, install XenoNet and run that line in a terminal. Or open **Network settings** there, choose **Join one**, and paste it, or the invite, the code and an address.

3. The new machine dials the one that made the invite by the key the invite names: through the network's relay, at the address given (`--at`, such as its LAN or tailnet address), or on your LAN (`--mdns`). The two prove the code to each other, the new one first. The inviting machine then signs it in, which asks you there (Touch ID on a Mac): once to add it, and twice more to pair the new machine's key into your identity, when the inviting machine holds your identity's root key, which the machine you started the network on does. The new machine checks the network's whole chain from its first entry, then reaches the other machines. It shows in every console within seconds.

A machine signed in by a window host that doesn't hold the root key joins without your identity: it takes part in the network, but nothing on it can act as you. Make invites on the machine you started the network on.

The inviting machine must be running when the new one joins. Five wrong codes retire the invite, and Network settings lists invites that closed unused. Adding a window host gives it a say in the network, so it needs the founder, or two window hosts: one made on another window host waits in the Network panel under **Waiting for approval**, with **Approve**, which asks you there.

If the network's relay lets only its own users in, the new machine isn't one yet: give it the inviting machine's address with `--at`, use `--mdns` on a LAN, or add it to the relay's allowlist.

## Relays

A relay carries traffic between two machines that can't reach each other directly: one behind a strict NAT, or a firewall that drops UDP between them. A network has none unless you name one, and then its machines reach each other on the LAN and by direct paths only. You can name:

- **your own** `iroh-relay`, by its URL ([Running your own relay](own-relay.md) says what one needs and how to set it up);
- **n0's public relays** (`n0`), which n0 runs and rate-limits.

Name one when you start the network, or later in **Network settings › Relays**. Adding a relay widens the network, so it asks you, takes the founder or two window hosts, and is done in the window; `xenonet net relay add` in a terminal says so. Taking one out is done at once, there or with `xenonet net relay remove <url>`. `xenonet net relay` lists them.

A machine can also have relays of its own, in no network's list, for its own dials: **Add as this machine's own**, or `xenonet net relay add <url> --mine`. A machine never dials a relay its networks don't list, whatever another machine's address names. A machine in several networks makes its home relay one that every network lists, so one network's machines never route through a relay another network chose.

A relay with a certificate from your own CA needs the CA on each machine, in `relay-ca.pem` ([own-relay.md](own-relay.md#a-relay-with-a-certificate-from-your-own-ca)).

## Sharing projects with a network

Each network sees the sessions in the projects you share into it, and nothing of the others. The first network a machine is in sees all its projects. A network after that starts with none: in its **Network settings**, under **What this network sees**, tick the projects, or **All of this machine's projects**. Sharing more lets that network's machines see more, so it's done in the window; sharing less works from your own terminal too:

```bash
xenonet net share none --network work
xenonet net share prj_1a2b…,prj_3c4d… --network work
```

A project counts once you've confirmed its checkout on this machine ([Projects](projects.md)). Claims and warnings go only to the networks the project is shared into.

## Mailboxes

A mailbox is a machine in the network that keeps what's sent to a machine that's away, sealed, and hands it over when that machine is back. It can't read what it keeps. It keeps a week at most, and up to 10,000 items or 50 MB for each machine, and it tells the sender what it took: the sender's `xenonet net status` and doctor show what's held, not delivered yet. A message that carries your presence (an Allow, say) never waits at a mailbox: it goes only to a machine that's connected.

Without a mailbox, a machine keeps what it has for a machine that's away and sends it when they're both connected again, for up to a week. Naming a mailbox isn't in the window or the command yet.

## What agents can do across machines

A session connected to XenoNet (see [Sessions working together](agents.md)) can do across the network what it does on one machine:

- **See the others.** `peers` lists this machine's sessions first, then each other machine's. Each row has `machine`, the machine it's on, and this machine's rows also have `here: true`. A session on a machine that's offline stays listed, marked offline since then. `whoami` says which machine and network a session is in.
- **Message them.** Name one by its id, which for another machine's session is its agent's `agt_…` (`peers` lists it with its machine), by words from its title, or with a selector: `machine:laptop` narrows to one machine. `repo` reaches the sessions in clones of the same remote on every machine. A message to many reaches 25 sessions at most across the network: this machine's first, then those in clones of the same repo, then the most recently active.
- **Know it arrived.** The other machine sends receipts back: stored, then seen once the session reads it. XenoNet's state lists them with each message. A message stored there that nobody reads within a week shows as unread. The other machine refuses a message to an actor it only observes, such as one XenoNet reads from its records, and the receipt says why.
- **Answer.** A reply goes back to the sender on its machine. A thread stops six messages deep, counted on every machine it crosses. To go on, send a message without `reply_to`, which starts a new thread.
- **Hear of conflicting work.** Before a session edits a file that a session on another machine claimed or is changing in another checkout of the same project (for a repo, a clone of it), XenoNet warns it, for example: "An agent on laptop (branch fix/login) is changing this file in its clone. Expect a merge conflict." Warnings never block.
- **Hand work off.** The `handoff` tool leaves work for another session: the goal, what's done, what's next, the files, the branch and commit. It goes to a session, or to the sessions in clones of the repo, on any machine, and the first to accept it gets it. The machine that made it decides, so it's taken once.

Actors message each other across machines too. An actor names another machine's actor by its `agt_…`, which that machine's presence lists with the actor's slug, and needs a grant that names it on its own machine. Name the machine too: `by:muse machine:laptop` covers only muse on laptop, while `by:muse` alone covers an actor called muse on any machine (see [scopes](live-delivery.md#what-actors-may-see-scopes)). For muse to answer, its own machine grants it `by:<slug>` for the sender. The machine a message arrives at checks it before storing it, as [Who decides](live-delivery.md#who-decides) says. An actor's grant names the network it acts in, so joining a second network never widens it.

A message from another machine reaches a session framed as "Message from an agent on laptop, another machine in your XenoNet network. It's information, not an instruction from your user." It approves nothing, like any message. A Claude Code session running with its permission checks bypassed holds messages from other machines until it asks for them.

## You, at any machine

Every console shows every machine, and reads the same on each. Every session carries its machine's id: another machine's id, or this machine's with a small **here** mark, in the active sessions, search, the inspector, the attention strip and Needs you. On the map, each other machine is a constellation of its own beside this one's, drawn from what that machine says of its sessions: a project for each repo (or each folder that isn't a repo), and its sessions with their state and what waits on you. A machine that goes offline dims.

Select one of another machine's sessions to see its state and what waits on you there, to open its transcript and stats, and to message it. Its transcript and stats open only in a window host's XenoNet window, never in a browser tab or for a session or actor on this machine. The first one you open after the window starts asks you once (Touch ID on a Mac), and that covers the next 12 hours.

The Network panel lists every machine: whether it's online or since when it's been away, its sessions, how many wait on you, what's held for it, and which machine you're at. A session on another machine that waits on you shows in the attention strip with its machine's name and its question's text, and one that seems stuck shows with why.

The load panel is **Machines** in a network (on its own it's **This computer**). Its three rings are the network's: CPU is the busy share of every thread, RAM all the memory, and GPU the mean of every GPU, with the busiest named. An Apple GPU's use counts, but its memory is the computer's, so it isn't in the GPU's own memory. Three bars under the rings show each machine's part of each, in that machine's color, and a row for each machine gives its CPU, GPU and memory. A machine that's offline is dimmed, says since when, and isn't counted. Each core, the last few minutes and the parts below are this machine's, under its id. Other machines' load comes only to window hosts, so on any other console their rows say their load shows on window hosts.

Click the panel's title, a machine's row, or **Machines** in the palette for the Machines view: the network's totals, a card for each machine with its load, its parts (each GPU on its own line) and its sessions by app, and a table to compare them. Its machine chips are the chips under Filters, so picking one there shows only that machine's work on the map too. It also filters by platform, by whether a machine has a GPU and by whether it's online, and sorts by name, CPU, GPU, memory or when a machine was last seen.

The chime and the notification for anything that waits on you, on this machine or another, play only on the machine where you last worked. Once you've been away from every console for five minutes, they play on all of them. A machine on its own always plays them.

From a window host's console you can:

- **Message any session, as you.** Choose **Message** beside it in the Network panel, or in a session's own panel. It arrives as "From you, at studio's console." A message as you to another machine is signed by your key on this one, which on a Mac asks Touch ID, once for each machine it goes to. The machine it reaches checks the signature against its own copy of your identity, and refuses a message as you without one.
- **Start a session on any machine**, as [Starting sessions](#starting-sessions) says.
- **Change who may start and wake sessions on another machine**, as [Who may start and wake sessions](#who-may-start-and-wake-sessions) says.
- **Answer a permission prompt** from a session XenoNet runs on any machine, as [Permission prompts](#permission-prompts) says.
- **Add a machine**, as above.
- **Remove a machine** that isn't a window host. Removing a window host needs the founder or two window hosts.

What changes one machine's own setup stays on it: connecting its sessions, installing its background service, its actor gateway and tunnel, and actors' grants and scopes. So do creating, joining and leaving a network, which XenoNet refuses from under an agent.

## Starting sessions

You, a session or an actor can start a Claude Code, Codex, Grok Build or Hermes Agent session on any machine in the network, in a folder there, with a first prompt. Claude Code and Codex run headless, through XenoNet's runner. Grok Build and Hermes Agent run over ACP (the Agent Client Protocol): XenoNet starts the agent with no window and none of its own credentials, opens a session in the folder, and holds it open for half an hour after each turn, so a message to it is its next turn. A Grok Build start with a permission mode runs headless, since ACP has no word for XenoNet's modes. A session's permission prompts over ACP come to Needs you, and an Allow covers that one call only. Each session shows on the map like any other, so you can message it and follow it.

- **In the console:** **Start a session…** on a machine's row in the Network panel, on its card in the Machines view, or in its node's panel on the map. Pick the machine, the app, the permissions (your own settings, or plan, accept edits, auto or don't ask), a folder from that machine's projects or its home, and write the prompt. **Start** is yours, and works a moment after the box opens.
- **Not in a terminal.** A start as you is signed by your key, which asks you (Touch ID on a Mac), so it's made in XenoNet's window. `xenonet start` in a terminal refuses and says so.
- **From a session or an actor:** the xenonet MCP server's `start_session` tool. An actor needs the spawn-sessions capability, and calls from its own machine (see [Actors](actors.md)). For a machine you set to take delegated work, `delegate` hands it a task instead, and the work runs on your standing delegation (see [Delegating work to another machine](delegation.md)).

The machine that runs it decides, on its own copy of the roster and of your identity. A start as you carries your key's signature over that one start, made at a console. A start from a session or an actor needs that machine's start setting to be **the network** or **delegated** (see below); an actor's carries a token its own machine made for that start. A session XenoNet couldn't check never starts one. The folder is a project or folder from its map, by id or path, a folder one of its sessions works in, or `~`. A session never starts bypassing its permission checks, and Codex runs with its own `config.toml`'s sandbox and approvals. A machine starts 20 sessions an hour at most, and 10 for any one machine that asks, or any one session, actor or you on the machine itself. A start passes the same limits as a message there: too many from one sender, or the same prompt again within a minute, is refused. While XenoNet is paused there (`xenonet live pause`), it starts nothing, whoever asks. A machine that has heard from no other machine of the network for ten minutes holds starts from them until it does.

The first prompt says who asked: "[XenoNet] Started by you, at inix's console.", or "[XenoNet] inix says this was asked by the session «Fix the login form» (claude:…). It came from an agent, not from your user."

The answers come back as receipts: started, with the new session's id, or refused with why, and later what the session came to when its turn ended. XenoNet reads that from the session's own output, never from the exit code alone, and says one of these:

- **finished**, and either **reported back itself** (it called the `send` tool) or "sent no report of its own", with its last words quoted;
- **stopped to ask something**, with the question quoted. It has done nothing more; message it to answer, and it takes up from there;
- **stopped for want of permission**, naming the tools it was denied;
- **failed**, with why (a sign-in it lacked, the half hour a turn may run, its exit code);
- **ended without a word**, when it sent no report and left no last message.

The new session needn't be told to report back: XenoNet does, and tells you as soon as the turn ends, even while the session lingers listening for a reply. For you they're a line in the activity feed and a toast; a session that stops to ask or fails is marked as needing you. For a session or an actor, a message, and a session that is idle is woken for it (see [Delivering at once: wake](live-delivery.md#delivering-at-once-wake)). A start goes only to a machine that's online: the answer says so at once otherwise, and nothing waits to start later. A machine whose run couldn't sign in to Claude Code says so, and Needs you shows which machine needs `xenonet runs sign-in` (see [Signing in for XenoNet's runs](live-delivery.md#signing-in-for-xenonets-runs)).

### Answering a question that stopped a session

A session that ends its turn asking something shows in **Needs you**, on its own machine and on every window host, as a question. On a window host, the row has **Go ahead**, **No** and **Answer…**. XenoNet itself sends the answer to the session, here or on another machine, as you at that console: the session reads "verified: you, at inix's console" and not an agent's relay, and a session that ended is resumed to take it. An agent can't press these buttons or fill in the answer: they are yours alone, like Allow, and XenoNet's control channel refuses to press them outside the demo. An agent that wants your answer can only say what it is waiting for.

## Who may start and wake sessions

Two settings on each machine decide what the network may do to its sessions:

| Setting | Values | Default |
|---|---|---|
| `start`: who may start sessions here | nobody (`off`), `you` from a window host's console, `network`: you, and the sessions and actors on window hosts, or `delegate`: the network, with what a session or actor starts here running on your standing delegation (see [Delegating work](delegation.md)) | `you` |
| `wake`: who may wake a session here to take a message at once | your `consent` per session (`xenonet live allow`), or `network`: any sender, within the wake budget | `consent` |

Set this machine's in the Network panel's **This machine** box. Narrowing is done at once, here or with `xenonet net set --start you` in your own terminal. Widening (`network`, `delegate`, wake `network`) asks you, so it's done in the window.

A session XenoNet starts, resumes or forks runs with the Claude Code sign-in you gave its runs on that machine (`xenonet runs sign-in`), and can read it from its environment. Under **the network** or **delegated**, the sessions and actors on window hosts can start such a session, or prompt one through a wake, and so can have it read the token. Widen these only on a machine whose token you'd let them see; the Allow card warns when a call may read the environment (see [Signing in for XenoNet's runs](live-delivery.md#signing-in-for-xenonets-runs)).

From a window host, change another machine's on its row in the Network panel. Narrowing (to nobody, you or consent) is done at once, and works from your own terminal too: `xenonet net set --on neo-rex --start off`. Widening to the network, or to delegation, asks you, so it's done in the window, and takes the founder's console or a second window host: from any other window host it waits for approval, shows on its row in every window host's Network panel, and another window host approves it there, which asks you there too.

## Permission prompts

A Claude Code session XenoNet starts, and a turn it resumes or forks, asks XenoNet, through the xenonet MCP server's `permission` tool, whenever its settings don't settle a tool call. XenoNet answers only for its own runs; any other session asks in its own app.

The prompt shows in Needs you on that machine and on every window host: "neo-rex · Fix the login form wants to run Bash", with what it would run, the machine and the run. It chimes where you are. **Allow** lets it run this once; **Deny** tells the session no. A call that may read the run's environment, where the sign-in token for XenoNet's runs is, says so on its row. Allow is yours, and works a moment after the row shows. It asks you once (Touch ID on a Mac), here or on any window host, and the machine the run is on checks it against its own copy of your identity. An Allow from another machine goes only while that machine is connected. Deny asks nothing.

Answers come only from XenoNet's window, never from a terminal or a session. An actor you gave `answer-prompts` may Deny, and Allow only the sessions and kinds of prompt you let it in the Actors panel, which asks you when you set or widen it. A run you delegated is answered by its machine within the delegation (see [Delegating work](delegation.md)). Nobody answering in ten minutes is a deny that says so; `XENONET_PERMISSION_HOLD_MS` sets another hold.

## A2A agents

When a machine runs its actor gateway with A2A on (see [Agents that speak A2A](actors.md#agents-that-speak-a2a)), an A2A agent reaches the network there. It's an actor on that machine, with its own token and scopes. A scope can name another machine: a grant of `machine:laptop` lets it reach the sessions on laptop, which it names by their `agt_…` ids in its message's `target`. Its message crosses to laptop like any other, and the session's reply there completes the agent's task. The actor gateway is that machine's alone: the other machines don't go through it.

## Services through the network

A machine can publish a local HTTP service to the network, and programs on the other machines reach it as if it ran on theirs. XenoNet carries each request and its answer straight between the two machines, and knows nothing of what the service is.

Publishing widens what the network can reach, so it asks you (Touch ID on a Mac) and is done in XenoNet's window on the machine that runs the service: **Network settings › Publish a service**. Give its name (`files`), its address (`http://127.0.0.1:8080`), its paths (`/api/v1/`) and, if you like, the machines that may call it. A service is published into one network, and only that network's machines reach it.

- The address is where the service listens on this machine: `127.0.0.1` or `[::1]` and a port, over plain http. XenoNet looks up no names, follows no redirects, uses no proxy, and refuses its own ports.
- The paths say what's reachable, as path prefixes that start and end with `/`. A request for any other path is refused before it reaches the service.
- Every machine in the network may call it, unless you name the machines that may.
- Publishing a name again replaces it. The Network panel's Services list shows what this machine publishes, with **Take off**; in a terminal, `xenonet svc list` and `xenonet svc unpublish files`. Leaving the network takes them all off.

Taking a service off asks nothing, and is yours from the window or your own terminal: XenoNet refuses it from under an agent. `xenonet svc publish` says to use the window.

On another machine in the network, a program calls its own XenoNet:

```bash
curl -H "x-xenonet-credential: $(cat "<state>/runtime/api-47840.cred")" \
  http://127.0.0.1:47840/api/svc/studio/files/api/v1/items
```

`/api/svc/<machine>/<name>` stands for the service's address, and the rest of the path is passed on: here the service on studio gets `GET /api/v1/items`. Like every route of XenoNet's API, it takes XenoNet's API credential: XenoNet writes it at each start to `api-<port>.cred` in the `runtime` folder of its state folder (see [Where XenoNet keeps files](how-it-works.md#where-xenonet-keeps-files)), readable only by your account, and a program sends it as `x-xenonet-credential`. Without it the answer is 401. Any program on that machine that can read the file may call it, with GET, HEAD, POST, PUT, PATCH, DELETE or OPTIONS. A web page can't: a request with an `Origin` header is refused.

The service gets:

- the method, the path, the query as it came, and the body. The path is checked against `--paths` once it's decoded, and one with `..`, `.`, `//`, `;` or `\` in it is refused;
- four of the caller's headers: `Authorization`, `Content-Type`, `Accept` and `User-Agent`. `Authorization` passes untouched, so the service's own tokens work, and XenoNet never logs it;
- `Host: 127.0.0.1:<port>`, and `Forwarded: by=xenonet;for=laptop`, which names the calling machine as its connection proved it. The caller's own `Forwarded`, `X-Forwarded-*` or `Via` never crosses, nor does any other header.

Its answer goes back with its status, headers and body, without `Set-Cookie` and the headers that only concern one connection.

A request's body and an answer are 8 MiB at most each, or the caller gets 413. Each machine may call each service 300 times a minute, then 429. A caller waits 90 seconds at most for the answer, then gets 504. When the machine that publishes it is offline, the caller gets 503 at once: a request is never kept for later, or written to disk on the way, since it may carry the service's credential. A machine passes on 8 requests from one other machine at once, and waits on 64 answers from one, so no single machine holds them all. XenoNet's own refusals have an `X-XenoNet-Refused` header with a code, such as `offline`, `not-published`, `not-allowed`, `path-refused` or `rate`, and a JSON body that says why, so a program can tell them from the service's own answers.

**A published service with no sign-in of its own is open to every program on every machine allowed to call it.** Give it tokens of its own, or name the machines that may call it.

### Xenolith between machines

Xenolith isn't a published service. It syncs spaces on its own protocol, `xenolith/sync/2`: on its own endpoint, or, in shared mode, through XenoNet's forward, which carries it to the machines of your networks for Xenolith once you allow its grant in **Network settings › Shared context**. [Sync between machines](../../xenolith/README.md#sync-between-machines) has the steps.

On the map, a project space is tied to its project by the project's id only when its folder's `xenolith-space.json` is signed by the context key Xenolith's grant names, and the space's owner owns the project. Any other space shows by its folder's name, and one about a project someone else owns says so ([the space marker](../../xenolith/FORMAT.md#the-space-marker)).

## When a machine is away

Each machine keeps working for its own sessions. Messages, receipts, claims and handoffs for a machine that's away wait for it: at a mailbox, if the network has one, or on the machine that sent them, for up to a week. They go once it's back, each delivered once. A machine that missed something fills the gap from any machine of the network that holds it, not only the one that sent it. Other machines' sessions stay in `peers`, marked offline, and the Network panel says since when.

## Removing a machine

In the Network panel, choose **Remove** on its row, or run `xenonet net remove desk`. From then on the other machines take nothing from it in that network and send it nothing, and its sessions leave every `peers`. It hears why ("Removed on … by studio"), and its doctor says to run `xenonet net leave`. It stays connected to the machines it shares another network with, for that network alone. The founder can't be removed.

A machine that was set up again keeps its id with **Replace**, which takes its old keys out and makes an invite for it to join again.

To take a machine out from the machine itself, run `xenonet net leave`. Its XenoNet forgets that network and carries on in any others, or alone. The founder leaves only with `--force`, since the others would lose it.

### A lost machine

Each machine you act from holds a device key of yours, which signs what you do there as you. When a machine is lost, revoke its device from another machine's window: **Network settings › Your devices**, then **Revoke** on that machine's device. It asks you once. Every machine in the network then refuses what that device signs, as you or with your presence, from then on. Then remove the machine from the network too, and **Replace** it if you set it up again. A machine can't revoke its own device, and `xenonet net revoke-device` only says to use the window.

## What crosses and what doesn't

Crosses, signed and sealed, to the machines of the network the project is shared into:

- each machine's sessions: id, app, title, state, the line each says it's doing, the project and branch, its folder with the home folder as `~`, when it was last active, and whether it waits on you;
- unless the machine shares titles only, each session's detail: the text of what waits on you (a question, or the permission it asks for), its last message (up to 280 characters) and why it seems stuck, with the home folder as `~`;
- when you last worked at each machine's console, so the chime plays where you are;
- claims and the files sessions are changing, as the project and the path inside it;
- messages, their receipts and handoffs;
- each machine's start and wake settings, what waits to widen them, and what its XenoNet takes;
- start requests, with their prompt, and their answers, to and from the machine they're for;
- a run's permission prompt, to each window host: its machine, session, run, tool and what it would run (a command or a path, up to 300 characters, with the home folder as `~`), and your answer back;
- a session's transcript or stats, only when you open one in a window host's XenoNet window. Nothing keeps them;
- each machine's load, every 5 seconds, to the window hosts connected to it: CPU, GPU and memory in use, and what the parts are (the CPU's model, threads and speed, total memory, each GPU's name and memory, platform and architecture). No machine names, serials or OS version. A machine keeps its load to itself with **Share load with the network** off in its Network panel (on by default), or `xenonet net set --no-share-load`;
- where each machine can be reached (its key, its addresses and its relay), to the network's other machines;
- a request to a service a machine published, and its answer, when a program on another machine asks for it (see [Services through the network](#services-through-the-network)).

Doesn't cross: file contents, any transcript you didn't open, sessions in projects you didn't share into that network, and collisions' diffs and MCP servers' details, which you open on their own machine. With **Share with the network: titles only** on in a machine's Network panel, its sessions' detail stays on it too, and so do its transcripts; its stats still open. The other consoles show that something waits there, and say the question is in that machine's console.

What crosses is for the consoles. It never reaches an agent: `peers` and the other tools show other machines' sessions without their questions, messages or stuck reasons, and XenoNet answers another machine's transcript or stats only to a window host's window. Only messages and handoffs reach agents.

A relay sees which machines use it, from which addresses, when and how much. A mailbox sees, for each item it keeps, which machine sent it, which it's for, its kind, size and time. Neither can read or change what they carry.

## Checking the network

- `xenonet net status` shows each network, this machine in it, its link, what's held for a machine that's away, and what waits for approval. `xenonet net hosts` lists the machines. `--json` gives each machine's path (`direct` or `relay`), how it was found and its clock's skew.
- `xenonet doctor`, under **Network**, checks:
  - the network's files and key, and that this machine is still in the network;
  - how it reaches the others: every machine connected, which through a relay, and which are away since when;
  - the other machines' clocks against this one's: a clock more than five minutes off has its presence dropped here, and its messages are still taken;
  - what a mailbox holds of this machine's, not delivered yet;
  - a machine's feed frozen because it signed two different things at one place, which you take again at the window;
  - a machine that sees a different roster;
  - each relay the machine has, the networks' and its own: whether it answers, how far away it is and whose CA vouched for its certificate, and `relay-ca.pem` when you added one ([own-relay.md](own-relay.md#checking-it));
  - whether the firewall lets other machines in ([The firewall](#the-firewall));
  - how this machine asks you before it acts as you (Touch ID, something weaker such as Windows Hello, polkit or a passphrase, or not at all), and a warning when your identity's root key is kept in a file here rather than sealed to the device key.

The link is `live` while this machine runs the network, whichever machines it reaches; each machine's row says whether it's online. `removed` means another machine took this one out; the reason says why. XenoNet keeps dialling machines that are away, from a quarter second up to every 30 seconds, and reaches a machine that comes back at once.
