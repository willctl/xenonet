# Waking actors when a message arrives

Status: built on `feat/actor-wake`, 2026-10-09 (`core/actors/wake.js`).

## Why

An actor learned of a message only when it asked. Grok Bot asked from an hourly routine, so an answer could take an hour. The owner asked for "near real time messages, so that actors don't need to poll".

## What each actor can take

| Actor | Can it be called? | Wake |
|---|---|---|
| A program of the owner's that stays running | It can hold a connection | **Listening**: `GET /api/actors/me/stream` (SSE), `xenonet actor listen` |
| Grok Bot | Its routines take a webhook trigger. The app keeps a key per routine and shows a Webhook URL (`…/automations/webhook/<id>` on Cursor's backend), a Webhook key and an Authorization header (read from its app bundle, 2026-10-09) | **Webhook** |
| Muse (7.0) | No trigger found: its `hatch://` links ignore everything but sign-in, and nothing in the app takes a webhook. It has recurring tasks | **Notify me**, and its own schedule |
| Any service with a webhook trigger | Yes | **Webhook** |

A wake to run a local command was left out on purpose. `xenonet actor listen --exec` covers a program that stays running, and a command XenoNet runs by itself would be a second way to run code that a window setting could change.

## Decisions

1. **A wake carries no content.** It says only which actor is woken and how many messages are waiting. It never includes what they say, who sent them, or a token. The actor reads them through its own authenticated route (the gateway's `inbox`, or `POST /api/engine/inbox`). A leaked wake URL, or a log at the receiving end, gives nothing away.
2. **The webhook request follows [Standard Webhooks](https://www.standardwebhooks.com).**
   - The body is `{ "type": "xenonet.inbox", "timestamp", "data": { "actor", "unread" } }`.
   - The headers are `webhook-id` and `webhook-timestamp`, plus `user-agent: XenoNet/<version>`.
   - A `whsec_` key gives a `webhook-signature` (`v1,` base64 HMAC-SHA256 of `id.timestamp.body` with the base64-decoded secret). This is checked against the libraries' own test vector.
   - Any other key goes as `Authorization: Bearer`, which a Grok Bot routine expects.
3. **The webhook's secrets stay in the credential store.**
   - The URL and the key are both kept there, because a URL can be a secret by itself, like Slack's.
   - XenoNet's database keeps only the host, whether the key signs, and when it was set.
   - Only the window, with its key, can set a webhook. Seeing, testing and turning one off are the owner's, from a terminal too.
   - Logs name the host, never the path, query or key. The doctor, which any local program can read, doesn't name even the host: it gives a reason such as "HTTP 401".
   - The store doesn't protect against programs running as the owner. On a Mac, an item made through `/usr/bin/security` is readable and writable by any of them, agents included. So `fire()` checks the stored URL again and calls it only if its host matches the one set in the window. Restricting the item's access list to XenoNet itself is a follow-up for `core/secret-store.js`, and would cover every secret XenoNet keeps.
4. **Requests are narrow.**
   - Only `https://`, or `http://` on this computer. No user name or password in the URL, at most 2,048 characters.
   - Nothing link-local: 169.254/16, fe80::/10 and their IPv4-mapped forms, checked in the URL and again in what its name resolves to before each call. A name that resolves differently by the time of the connection could still get through; covering that needs the address pinned in the connection itself.
   - No redirects followed, a 10 s timeout, and the response body discarded.
5. **Wakes are paced.** Any agent can write to an actor, and a wake may start a run the owner pays for.
   - Messages within 1.5 s make one wake, and an actor is woken at most every 30 s and 20 times an hour (5 at once).
   - An actor isn't woken again while the messages from its last wake are still unread, for up to 10 min.
   - One wake at a time per actor: a message that arrives while one is on its way waits for the next.
   - A failure backs off 30 s, 2 min, 10 min, then 30 min.
   - All wakes together are capped at 120 an hour.
   - A program that's listening needs no webhook, so a message reaches it over the stream instead.
   - The owner's Test is limited to one every 5 s, and holds nothing back.
6. **Streams are an actor's own call.**
   - The token and the process check apply as on its other routes.
   - Each actor can hold at most four streams.
   - A stream re-checks the token every 20 s and ends with a `closed` event once the token stops working.
   - `xenonet actor listen` waits out restarts, but stops on a refused token.
   - The program `--exec` runs gets the message on stdin, never the actor's token. The message is untrusted input, since any agent can write it.
7. **Notify me is the default for an actor connected through the gateway**, since nothing else wakes it.
   - The notification comes once a message has waited 2 min, at most hourly per actor.
   - It gives the count, never the sender or the words.
   - The demo stays quiet.

## Tests

- `test/actor-wake.test.js`: the signature vector, URL rules, batching and gaps, backoff and the words for each failure, secrets in the store and out of logs, Notify me, streams and the stream route.
- `test/cli-actor.test.js`: `listen`, `listen --exec`, `wake`.
- `test/e2e/actor-requests.test.js`: a webhook set in the window, tested, then called within seconds of a message through the gateway.
