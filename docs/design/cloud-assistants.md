# Remote actors: reading through a connection

A remote actor, such as an OpenAI dot, reads what's going on on one
machine through a connection the person pairs in XenoNet's window. The
connection holds the actor grant every actor holds
([authorization.md](authorization.md), "The actor grant"), issued with the
read right and scoped to the projects the person picked. It is
experimental: fixture, local-client and synthetic-data checks pass, and the
checks through a real tunnel with a real dot haven't been run (section 8).
The user guide is [Remote actors](../guide/cloud-assistants.md).

## 1. What it is

Through a connection, the actor sees the sessions in the projects the
person picked (statuses, times, counts, attention items, other actors'
relations there), and nothing else. A connection's grant carries the read
right only (`xenonet_actors::connections::RIGHTS`), so it changes nothing.
Sending, answering, starting and claiming are rights of the same grant,
which an actor gets by registering
([actors/interface.md](actors/interface.md)); a connection never widens
to them.

The path a read takes:

```
cloud assistant (a dot)
  -> OpenAI-managed tunnel endpoint
  -> tunnel-client, on the person's machine
  -> xenonet mcp assistant --grant <grant>      crates/xenonet-mcp/src/assistant, stdio
  -> private endpoint, 127.0.0.1               crates/xenonet-service/src/assistant_endpoint.rs
  -> assistant.read                            crates/xenonet-actors/src/assistant/service.rs
  -> XenoNet's in-memory state                 crates/xenonet-actors/src/assistant/source.rs
```

The local HTTP API, the component service and the other MCP servers are
unreachable from a connection.

## 2. Decisions

| Question | Decision | Why |
| --- | --- | --- |
| Local transport for the adapter | A separate private endpoint, not a role on the component service | The component service's hello takes the Tools token first, which the adapter must never hold. A separate listener with its own fresh allowlist (`hello`, `assistant.read`) leaves the Desktop and Tools roles' code untouched, and works in XenoNet Dev, where there's no component service. It reuses the component service's framing (`crates/xenonet-service/src/rpc.rs`), with a line limit of its own (320 KiB). |
| When it listens | Only while a grant could be used | No listener at all for an owner who never pairs anything. It stops when the last grant is revoked or expires and when access is turned off. |
| What the connection holds | The connection's actor grant ([authorization.md](authorization.md), "The actor grant"): a Biscuit minted at this machine with the right to read, written `xenonet_actor_` and base64url, kept for the adapter in `<state>/assistant/credentials/<grant>.json`, readable only by the person's account. The store keeps only its revocation id | One grant for every actor, so the assistant's reads are decided by `xenonet-authz` like any actor's. The adapter needs the token to say hello, and an owner-only file is what protects actors' token files on every supported OS. A file another account can read is deleted and pairing fails. |
| Revocation without the service | Tombstones in `<state>/assistant/` (`disabled`, `revoked/<grant>`) | They're honored by the service on every read and every two seconds, by the adapter before it serves anything from its cache, and on startup, where a tombstone wins over a store restored from a copy. |
| Who may do what | Pairing, widening a grant, rotating and turning access back on: the window only, and pairing and widening take the owner's presence (Touch ID on a Mac). Listing, revoking and turning access off: the window or the owner's own terminal (the local API's `You` rows) | Anything that creates or widens access needs the window and the owner there. The fail-safe directions should work from a terminal, without the window, so an owner can stop access quickly. |
| Audience | The owner says who can call the endpoint, and whether they checked. Until they say they checked, the grant reads made-up data only (`data_mode: fixture`) | The tunnel passes XenoNet no assertion of the caller, so XenoNet can't check the audience itself. Recording the owner's statement, and keeping real data off without it, follows the spec's rule that real data stays disabled until the audience is bounded. |
| Account links | Unverified, or the owner's word for an identity XenoNet already knows; never verified through this route | No transport here provides a provider assertion to validate. The verification path exists (`crates/xenonet-actors/src/account_links.rs`, `adapters.rs`) and is tested with made-up evidence, so a later adapter with a real assertion plugs in without a new table. |
| Identity in snapshots | `identity` stays null for an owner-asserted link; `remote.accountLink` says how sure XenoNet is | Readers take `identity` as fact, so a link that's only the person's word never goes there. |
| Presence | A read is working for a 90-second lease, then unknown | A read is evidence of activity at that moment only. The lease ages to unknown, never offline, and the most present instance still wins, so an observer's word about Grok Bot outranks a lapsed lease. |
| Pagination | A page cursor is the connection's grant narrowed to one read, the hash of its tool and filters and five minutes, holding where the last page ended and the grant's revision (`crates/xenonet-service/src/compose/assistant/pages.rs`) | No server-side snapshot to keep or expire, and no key of its own: a cursor can't be made up or changed, and it ends with the grant. The view is documented as the current one: an item that changes between pages can move. |
| Status of a session | From the tailer's signals: waiting (question or permission), stuck, working (a turn under way), completed (a turn ended and hasn't been looked at), idle, unknown (its source unavailable) | These are the signals the console already shows, so the answer matches what the owner sees. `status_basis` says which signal it came from. |

## 3. The actor model

A connection belongs to an actor in the `actors` table; there is no
second registry. The store (`xenonet.sqlite`, `crates/xenonet-store/src/engine.rs`) holds its child tables:

- `actor_connections`: one per way an actor reaches XenoNet (transport,
  label, audience and the owner's word on it), cascading with its actor.
- `actor_grants`: what a connection may read (labels, lookback, data
  mode), until when, its revision, and its token's revocation id. The
  projects it reads are its grant's scope row in `grant_scopes`, keyed by
  the same id: the store's own `prj_` ids, the ones every other surface
  names projects by. A project is read through its confirmed checkouts
  on this machine (`project:<chk_…>` on the map); a candidate checkout or
  a zone can't be picked, since it gives rights in no project.
- `actor_account_links`, `actor_external_refs`: how sure XenoNet is of the
  account, and the provider's own ids once proven, unique per namespace;
  a conflict is refused for the owner to review, never merged.
- `assistant_meta` (the device's id and a fingerprint of the installation),
  `assistant_audit` (seven days, 20,000 records).

Foreign keys are on in the store. Remote-only actors have
`via: ["remote"]`, no `allowedAt`, no capabilities and no heartbeat, so
they don't count as registered, don't show an expired token, aren't
message-reachable, and don't take one of the 16 registered places.
`unregister` keeps an actor that still has remote connections. Removing an
actor revokes its grants first (`Connections::before_remove`), then the
rows go with it. `add_via` and `set_remote` change how XenoNet knows an
actor without touching its name, capabilities or identity.

A product has `remote` beside `registers`
(`crates/xenonet-adapter/src/products.rs`): dots, Grok Bot and Hermes
Agent can be paired; none of them registers because of it. A new dot is
`actor:dots`, then `actor:dots-<4 hex>`; `product_of_slug` reserves
`dots-…` for dots, as it does `hermes-…` for Hermes.

## 4. The trust boundary

What the owner approves is a grant, and what holds it is the grant's token
in a file readable only by the owner's account. The endpoint accepts a
hello only with the exact keys `role`, `api`, `grant_id` and `token`; a
wrong token is refused like an unknown grant, and the right one hears how
its grant stands (revoked, expired, off). After hello, only
`assistant.read` is answered. The service, not the adapter, decides the
grant on each read with `xenonet-authz`, and again just before the answer
goes: revoked, expired or off by then, the answer is dropped; changed in
scope, the read runs again once under the new revision.

What completes the boundary is outside XenoNet: the tunnel's configuration
and its audience, which the owner sets up with OpenAI and attests to.
Holding the grant's token says nothing about which remote model is calling.

A state folder copied to another machine or account doesn't match the
fingerprint of the installation that made its grants, so they're revoked
(`device-changed`) and the device gets a new id.

## 5. Data policy

Every answer is made afresh from named fields
(`crates/xenonet-actors/src/assistant/projection.rs`) and checked against strict
schemas (`assistant/result/v1`, `assistant/error/v1` and
`actor/connection/v1`, typed in `crates/xenonet-model`) before it goes. These are the only strict
schemas in XenoNet: data that leaves the machine is an allowlist.

- A grant sees only its places, and filtering, counting, sorting and
  paging all happen on that view. A session or project it doesn't see is
  `NOT_FOUND`, the same as one that doesn't exist.
- A collision is shown only when the grant sees both sessions.
- Labels are null unless turned on, then cleaned: control and direction
  characters, key-like runs, emails and folders named for a person go.
- Freshness: `live` within 30 seconds of the oldest relevant source's
  observation, `stale` after, `unknown` with none; a degraded or
  unavailable source makes the answer `partial`, with a warning naming it.
  A session whose source is unavailable is `unknown`, never `idle`.
- Stats only with `include_stats`; input counts fresh input plus cache
  writes, and cached input apart, so nothing is counted twice.
- Errors are fixed codes and sentences; the precise reason goes only to
  the local log.

## 6. Logging

`assistant_audit` keeps, for each hello and read: time, request id,
grant, revision, actor, connection, operation, a result code, latency,
bytes and how many projects the grant could see. The owner's own pairings,
changes, rotations, revocations and switches are kept beside them, marked
as the owner's. Never a title, name, argument, credential, path or
account. `xenonet assistant audit` and `GET /api/assistant/audit` read it.

## 7. Threat model

| Threat | Control | Tested in |
| --- | --- | --- |
| A remote caller with no grant | The tunnel's own access, then the endpoint's hello; a wrong token hears nothing more than an unknown one | `crates/xenonet-service/tests/assistant_endpoint.rs` |
| A caller asking for an unselected project or session | Authorized view before any filter or count; uniform `NOT_FOUND` | `crates/xenonet-service/src/compose/assistant/reads.rs`, `projection.rs` |
| A compromised adapter asking for more | Fresh method allowlist on the endpoint; every check in the service; the component service refuses the assistant role, and the endpoint refuses Tools tokens | `assistant_endpoint.rs` |
| Prompt injection or secrets in titles | Labels off by default; cleaning when on; tool descriptions and instructions say labels are data, never instructions | `projection.rs` (canaries) |
| Stale authorization, replay | Recheck before release; revision-bound cursors and cache; tombstones; the grant's file deleted on revoke; removing a registered actor revokes its connections | `assistant_endpoint.rs`, `pages.rs`, `crates/xenonet-actors/src/connections.rs` |
| A schema change adding fields | Strict output schemas checked before release | `crates/xenonet-model` (strict types) |
| Overload | 16 KiB arguments, 256 KiB answers, 60 a minute with bursts of ten, four at a time, ten seconds, 320 KiB lines, refused hellos throttled | `assistant_endpoint.rs` |
| A copied state folder | Installation fingerprint; grants revoked on mismatch | `crates/xenonet-actors/src/connections.rs` |
| Cached data after revocation, expiry or a narrower scope | The adapter serves a cached answer only while the grant's files say it stands (its token's file, tombstones, expiry) and only under the revision on disk now | `crates/xenonet-actors/src/assistant/credentials.rs`, `assistant_endpoint.rs` |
| A stale endpoint file after a crash, naming a port someone else took | The adapter connects only while the process that wrote the file runs as this account | `assistant_endpoint.rs` |
| Work piling up past the concurrency limit | A read that timed out keeps its slot until its work ends | `crates/xenonet-actors/src/assistant/service.rs` |

What this can't stop: code running as the person's account can read the
grant's file, the store and the transcripts themselves, and change the
tombstones. The design keeps a remote caller out of that category; it
doesn't claim to protect against a compromised account.

## 8. Checks

| Check | Where it stands |
| --- | --- |
| G0a: adapter initialize, tools/list, tools/call locally, against the fixture and a real grant | Covered by the MCP golden files (`crates/xenonet-mcp/tests/golden/the-assistant-*.json`) |
| G0b: the tunnel calls the fixture adapter | Not run: needs the person's tunnel, credentials and approval |
| G0c: the actual dot lists the tools and calls `xenonet_status` | Not run |
| G0d: the dot sees a fixture change with new times | Not run; the fixture changes every two minutes for it |
| G0e: a refused request, and local revocation stopping reads | Covered locally (`assistant_endpoint.rs`, `test/e2e/assistant.test.js`); not through the tunnel |
| G0f: the endpoint's audience bounded and verified | Not run: the person's to check in OpenAI's settings |
| T7, T8 (real dot, real data) | Not run |
| End to end in XenoNet Dev's test instance | `test/e2e/assistant.test.js` |

Docs must not claim dot support until G0c and T7 pass on the actual
surface. A test in local Codex, ordinary ChatGPT, an MCP inspector or the
Responses API doesn't establish a dot's compatibility.

## 9. Turning it off

`xenonet assistant disable` turns every connection off; then revoke the
tunnel with OpenAI. The tombstones keep revoked grants revoked even if the
store is restored from a copy.

## 10. Not built

Transcript access, a hosted gateway, aggregating several machines through
one connection, and any right beyond read through a connection. The device
id is in every answer so a gateway can aggregate later without a new
contract.
