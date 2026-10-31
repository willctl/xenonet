# Xenolith across machines: one home, many satellites

Status: design, reviewed, not built. Xenolith stays standalone: nothing here imports or calls another program.

## Problem

A space is synced by hand today: on the curator you grant, make a token and publish the port; on the other machine you add the peer and follow each space by name. That is five or six commands per pair, and a token to carry between machines. Each machine also runs its own curator, so two machines can end up filing notes for spaces nobody chose to split.

## What it should be

- The agentic parts (the curator that files notes into pages, and Ask) and the authority for pages run on **one machine**. A second curator on a space is a bug.
- Every other machine reads and leaves notes, and its tools work the same.
- One step per extra machine, over any network, not shaped around one person's setup.

## Model

- **Home:** the one machine that runs the curator and Ask, with the curator's sign-in. It is the authority for pages.
- **Satellite:** any other machine. It keeps a full local copy, so `context_start`, search and reads are local. Sessions leave notes as usual. Its curator and Ask are off. It pushes notes to home and pulls pages, removals and the recent log, as the follower sync does today.
- A machine is home or a satellite as a whole. A satellite follows **every space in home's space list that has no explicit follow record of its own**, including spaces made later. The per-space `xenolith sync follow` stays, for splitting spaces between homes on purpose.

## Pairing

On home:

```bash
xenolith pair offer          # prints a code; valid 10 minutes, one join
```

On the satellite:

```bash
xenolith pair join --url https://home.example/xenolith   # asks for the code
```

- The code is `<home-id>.<secret>` and nothing else. The address is a separate `--url`, so a code pasted in a chat reveals no address. The code is read from a prompt or stdin, never an argument.
- `join` makes a keypair, sends its public key and an HMAC of it under the secret to `/sync/v1/pair`, and home answers with a machine token encrypted to that key. A code seen by someone else is useless without the secret and the race: home refuses a second join on the same code outright and says who joined and from where. `/sync/v1/pair` shares the bad-token rate limit.
- The token is `ctx:read:all` plus `ctx:append:all`. Writes to pages are not part of pairing: replacing pages from a satellite is a later `xenolith grant` the owner runs by hand. `machine:local` on home stays out of `all`, as it is now.
- The token is stored as today in `peers/<name>.token` (0600, or a protected ACL on Windows). The satellite then turns its curator and Ask off and sets home as the peer. `xenolith sync status` names home.
- `pair offer` needs the owner key, as deciding held changes does. Pairings and mode changes go in the audit chain on both machines with the token's id, never the code or the token.

## Transport is the person's choice

Xenolith talks to home's `/sync/v1/` at any base URL: `https://` anywhere, or loopback `http://`. A LAN or VPN address, a reverse proxy that strips its prefix, or a forwarder on this machine all work, and Xenolith doesn't know which. Programs that forward ports can add a one-step action of their own around `xenolith pair offer --json` and `xenolith pair join`; those are documented in their own docs, not here.

## Ask on a satellite

Ask runs on home. A satellite sends it over its token, which needs a separate `ask` scope: `ctx:read` must not let any reader drive an agent that spends the owner's quota. The scope is rate-limited per token. When home is unreachable Ask says so, and search still works locally.

## Home is away, or moves

- No automatic election: two curators on one space is the failure to avoid, and a partition can't tell away from gone. A satellite works while home is away: reads are local, notes queue and go when home is back.
- The first version has no `home move`. To move home, pair again on the new machine; the satellite sees a new curator id and refuses until it is re-paired, as the curator-id pin does today.
- Later: `home move`, run on the old home, signs a handover (new id, epoch, address) that satellites accept by themselves, and an unsigned `home take` that bumps an epoch into every curator id and token so a returning old home is refused ("superseded": curator off, its inbox re-queued to the new home).

## Not synced

Per machine, never copied: `access.json`, tokens, the owner key, the curator's sign-in, `config.json`, the audit chain, tray settings.

## Conflicts

Unchanged: each entry is taken once, and a page that changed on home since the satellite last pulled it makes the satellite's replacement a proposal that waits for the owner.

## First version

One PR: `pair offer` and `pair join` with `--url`; the keypair-bound token; append and read scopes only; follow-all as "every space in home's list without an explicit record"; `sync status` showing home; curator and Ask off on a satellite. No page writes, Ask forwarding or `home move`.

## Open questions

1. May a satellite run its own curator on a private space home doesn't have, or must every space live on home?
2. Windows home: the service runs as a service account and the curator as the person, so the owner key that authorizes `pair offer` is the person's, on home only.
