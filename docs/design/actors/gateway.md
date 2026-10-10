# The actor gateway: tunnels, checks and reconnecting

Status: built on `feat/actors-talk`, 2026-10-08. The gateway itself
(`server/actor-gateway.js`) is described in `interface.md`; this note
covers how it gets on the internet and stays there.

## Why

The first live run between Grok Bot and Muse (2026-10-08) worked, but
setting it up took six manual steps across four apps:

- turn the gateway on;
- run `tailscale funnel` in a terminal;
- make a token per actor;
- paste each token into an app that hides its settings.

It also failed quietly. Homebrew's `tailscale` (1.98.8) set Funnel up
against a newer running Tailscale (1.104.1), and the name never reached
public DNS. Nothing said so.

It also assumed things that don't hold for everyone who runs XenoNet:

- that Tailscale is there;
- that the address never changes;
- that someone notices when a reboot, a rename or a token expiry breaks the
  connection.

XenoNet is a public project, so the gateway can't assume any one way onto
the internet.

## Decisions

1. **No tunnel is assumed.** The owner picks one of five kinds
   (`server/gateway-tunnel.js`). XenoNet finds which tools are installed and
   offers all five:

   | Kind | XenoNet runs | Address | Why it's here |
   |---|---|---|---|
   | `tailscale` | `tailscale funnel --bg --yes <port>`, once; Tailscale keeps it | the computer's tailnet name | many owners already have it |
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
4. **What Funnel already serves is left alone.** Before it runs anything,
   XenoNet reads `tailscale funnel status --json` for port 443 of the
   computer's name:
   - **nothing there**: XenoNet sets it up;
   - **this gateway**: nothing to do;
   - **only `/`, pointing elsewhere** (another XenoNet, an old port): XenoNet
     only replaces it on **Repair**;
   - **other paths too**: XenoNet refuses, since `funnel` would put those on
     the internet with the gateway.

   Turning the gateway off turns Funnel off only when it serves the gateway
   alone.
5. **Checked from the internet side.** `server/gateway-reach.js` first
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

   Checked on this Mac on 2026-10-08:
   - through Funnel, it passed;
   - with the gateway down, it said 502.
6. **It comes back by itself** (`server/gateway-control.js`). The settings
   `{ on, host, tunnel: { kind, name, host } }` are kept in the engine's
   database. At start XenoNet applies them again. A one-minute clock then
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
   the engine's meta). When the address in use differs from that host, the
   actor's row and `xenonet doctor` say its app has an old address. The
   app's token still works, since tokens don't depend on the address. **New
   address…** shows what to tell it.
8. **Connections that are about to break are said in time:**
   - a gateway token within a week of its 30 unused days;
   - calls with a token that was replaced, taken away or expired, with
     the actor named from the revocation record.
9. **What to tell each app, written out** (`core/actors/handoff.js`).
   - **Grok Bot** is set up from its chat, and its MCP header can't use a
     saved secret, so its message carries the token. The guide says to
     delete that chat afterwards.
   - **Muse's** message never carries the token: Muse asks for the key in
     its connector card's own field.
   - **Any other MCP host** gets the URL and the header.

## What stays the owner's

All of the following are owner actions, checked by `fromYou` in
`server/routes/actors.js`: picking a tunnel, turning the gateway on or
off, Check, Repair and `revoke`. Making a gateway token is the window's
alone.

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

- `test/gateway-tunnel.test.js`:
  - Tailscale with stand-in commands: version matching, Funnel's
    states, the link Tailscale asks you to visit;
  - the lines each tool writes;
  - a child's restarts and failure;
  - the check's every answer;
  - the handoff text.
- `test/actor-gateway.test.js`:
  - the routes;
  - the one-time word;
  - an address change;
  - a refused token;
  - a restart that brings a quick tunnel back with a new address;
  - a wake.
- `test/e2e/actor-requests.test.js`: setting up in the window, with
  `XENONET_GATEWAY_PROBE=local` checking through the gateway's own port
  instead of the internet.
- `npm run test:real`: Grok Bot and Muse, for real.
