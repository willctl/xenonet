# The actor gateway: tunnels, checks and reconnecting

How the actor gateway gets on the internet and stays there. The gateway
itself (`crates/xenonet-service/src/gateway/actor_gateway.rs`) is
described in [interface.md](interface.md).

## Why

Without help, connecting an actor such as Grok Bot or Muse takes manual
steps across several apps: turning the gateway on, running a tunnel in a
terminal, making a token per actor and pasting each into an app that hides
its settings. A tunnel can also fail quietly: a `tailscale` command older
than the running Tailscale can set Funnel up against it and leave the name
out of public DNS, with nothing saying so. And the gateway can't assume
that Tailscale is there, that the address never changes, or that someone
notices when a reboot, a rename or a token expiry breaks the connection.

XenoNet is a public project, so the gateway can't assume any one way onto
the internet.

## Decisions

1. **No tunnel is assumed.** The owner picks one of five kinds
   (`crates/xenonet-service/src/gateway/tunnel.rs`). XenoNet finds which tools are installed and
   offers all five:

   | Kind | XenoNet runs | Address | Why it's here |
   |---|---|---|---|
   | `tailscale` | `tailscale funnel --bg --yes --set-path /mcp http://127.0.0.1:<port>/mcp`, once; Tailscale keeps it | the computer's tailnet name, `/mcp` alone | many owners already have it |
   | `cloudflare` | `cloudflared tunnel --no-autoupdate run --url http://127.0.0.1:<port> <name>`, as its child | a name on the owner's domain | stable, free, own domain |
   | `cloudflare-quick` | `cloudflared tunnel --no-autoupdate --url …`, as its child | random, new at every start | nothing to sign up for |
   | `ngrok` | `ngrok http 127.0.0.1:<port> --log stdout --log-format json [--url https://<domain>]`, as its child | the owner's ngrok domain | free static domain |
   | `own` | nothing | the name the owner gives | anything else: Caddy, nginx, frp, a VPS |

2. **XenoNet never holds a tunnel's secrets.** Each tool reads its own
   sign-in from its own config: `~/.cloudflared`, ngrok's
   `ngrok.yml` or tailscaled. XenoNet only starts and stops the tool. This
   leaves out Cloudflare's dashboard-managed tunnels, which run on a
   token; an owner with one can run it as a service and pick `own`.
3. **The command that matches the running Tailscale.** XenoNet looks for
   Tailscale on PATH, in the Mac app and in Homebrew's folders. It uses the
   copy whose version is the daemon's (`status --json` → `Version`).
4. **Funnel publishes `/mcp` alone, and what it already serves is left
   alone.** A2A stays off the internet. Before it runs anything, XenoNet reads
   `tailscale funnel status --json` for port 443 of the computer's name:
   - **nothing there**: XenoNet sets it up;
   - **this gateway's `/mcp` alone**: nothing to do;
   - **this gateway's whole port** (`/`, as XenoNet set it up before
     `/mcp` alone): XenoNet turns it off and sets `/mcp` up, without a
     Repair;
   - **one path, pointing elsewhere** (another XenoNet, an old port):
     XenoNet only replaces it on **Repair**;
   - **other paths too**: XenoNet refuses, since `funnel` would put those on
     the internet with the gateway.

   The tunnel's view lists the paths Funnel puts on the internet
   (`public`), the panel says them, and the doctor warns when they're more
   than `/mcp`. Turning the gateway off turns Funnel off only when it
   serves the gateway alone.
5. **Checked from the internet side.** `gateway/reach.rs` first
   asks public resolvers (1.1.1.1, 8.8.8.8, 9.9.9.9) for the name, so it
   gets the internet's answer rather than what MagicDNS or a hosts file
   says. It then sends `GET https://<name>/mcp` to that address, carrying a
   one-time word. The gateway answers that word with 204 once, within 30 s,
   and answers nothing else that way. So a pass means this XenoNet's gateway
   answered through the whole tunnel. A failure is said in words:
   - no DNS name yet;
   - DNS unreachable;
   - nothing answering;
   - the wrong certificate;
   - Host rewritten (421);
   - the tunnel up but the gateway not reached (502–504);
   - something else answering.

6. **It comes back by itself** (`gateway/control.rs`). The settings
   `{ on, host, tunnel: { kind, name, host } }` are kept in the store. At start XenoNet applies them again. A one-minute clock then
   checks the address:
   - every ten minutes while it passes;
   - every two minutes while it fails;
   - at once when the clock jumped (the computer slept).

   Before each check, XenoNet sets up Funnel again if it went. A child tunnel
   that exits is started again after 5, 20 and 60 s. After a fourth exit in
   ten minutes it stays failed until the next check. A child's pid is kept
   in the state folder. A child left behind by a XenoNet that didn't stop
   cleanly is stopped first, but only if it is still that tool, because
   ngrok refuses a second agent on the same domain.
7. **An address change names the apps it strands.** The gateway records
   each actor's last call and the public host it came by (`gateway.seen` in
   the store's meta). When the address in use differs from that host, the
   actor's row and `xenonet doctor` say its app has an old address. The
   app's token still works, since tokens don't depend on the address. **New
   address…** shows what to tell it.
8. **Connections that are about to break are said in time:**
   - a gateway token within a week of its 30 unused days;
   - calls with a token that was replaced, taken away or expired, with
     the actor named from the revocation record.
9. **What to tell each app, written out** (`crates/xenonet-actors/src/handoff.rs`,
   and `src/shared/handoff.js` for the page).
   - **Grok Bot** is set up from its chat, and its message never carries
     the token: it asks Grok Bot to request the token with its secure
     secret card, whose value Grok Bot's model never sees, and to send it
     as the MCP header.
   - **Muse's** message never carries the token: Muse asks for the key in
     its connector card's own field.
   - **Any other MCP host** gets the URL and the header.

## What stays the owner's

All of the following are owner actions, the `You` rows of the local
API's rules table (the window, or your own terminal): picking a tunnel,
turning the gateway on or off, Check, Repair and `revoke`. Making a
gateway token is the window's alone, and asks your presence (Touch ID on a
Mac), since it widens the actor's grant.

A gateway token is the actor's grant with the right to use the gateway,
narrowed to the gateway's door and written `xenonet_gateway_` and
base64url ([authorization.md](../authorization.md), "The actor grant").
It works at `/mcp` and `/a2a` and nowhere else, and XenoNet keeps only its
revocation id.

## Asking for a token

An agent can start a connection without ever holding its secret. The ask
(`ask_gateway` in `crates/xenonet-actors/src/registration.rs`,
`POST /api/actors/<slug>/gateway/ask`, the
`ask_gateway_token` MCP tool and `xenonet gateway token`) is open to any
program on this machine, as registering is, because it holds nothing:

- It makes no token and the answer (`actor/gateway-ask/v1`) has none. The
  MCP tool also withholds anything shaped like a token from what it
  returns, so a change on the server's side can't put one in a transcript.
- It leaves a card in the window (the attention strip and the Actors
  panel) with who asked. **Allow**, which only the window can send, asks
  your presence and makes the token then, as Connect… does, and the answer
  to the window alone holds it, once. Deny drops the ask.
- Until Allow, nothing changes: the old token keeps working, so an ask can't
  break a connection. An ask lapses with a registration's ten minutes, one
  waits per actor, four at a time and twenty an hour.
- It's refused from an actor's token, and from under an actor product's own
  processes (the verifier's `asker`), which could be relaying for its cloud. A
  session, or your own terminal, may ask.
- The token shows masked (`src/secret.js`): out of the page until Show,
  ten seconds, masked again in a screenshot or when the window loses
  focus. The driver and the console tools list asks and have no method to
  allow one.

Why the click stays: `x-xenonet-agent` is a header any local program can
send, so a route that made tokens for it would let anything that can pose
as an agent mint a credential that works from the internet. Asking costs
a card, and the secret still only ever exists where the person is looking.
A delivery that needs no click would need somewhere safe to put the token
(a keychain item the agent can't read, say), which this doesn't build.

## Still open

- **Sign-in instead of tokens.** MCP's authorization (OAuth 2.1 with
  dynamic client registration) would let an app connect by URL, send the
  owner to an approval in XenoNet's window, and renew its own access.
  Nothing would be copied, and a new token wouldn't strand an app. Muse's
  connectors (`direct_oauth` with a registration URL) and Grok Bot's URL
  servers both seem to support it, judging from their code. Neither has
  been tried against XenoNet. It needs the gateway to serve the
  authorization endpoints, with a consent card like a registration
  request's.
- **An address that outlives the computer.** A stable name today comes
  from the tunnel: a renamed tailnet machine, a Cloudflare name, an ngrok
  domain. Moving XenoNet to another computer still means a new address for
  each app, unless the owner's own name points at the new one. A relay
  XenoNet connects out to would avoid that, at the cost of a service someone
  has to run.

## Tests

- `crates/xenonet-service/tests/tunnel.rs`: Tailscale with stand-in
  commands (version matching, Funnel's states, the link Tailscale asks you
  to visit), the lines each tool writes, a child's restarts and failure,
  and a tunnel left running stopped only while it is still that tool.
- `crates/xenonet-service/tests/gateway.rs` and `gateway_control.rs`: the
  routes, the one-time word, refused tokens, the gateway's port, and on
  and off with a tunnel of your own.
- `test/gateway-tunnel.test.js`: the handoff text the page shows.
- `test/e2e/actor-requests.test.js`: setting up in the window, with
  `XENONET_GATEWAY_PROBE=local` checking through the gateway's own port
  instead of the internet.
- `npm run test:real`: Grok Bot and Muse, for real.
