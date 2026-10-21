# Remote actors: read-only participation

Status: built, and experimental, from the proposed
specification of 7 October 2026 ("XenoNet remote actor integration
development specification"). Fixture, local-client and synthetic-data
checks pass on Windows. The real tunnel and dot checks (G0b to G0f, T7,
T8) haven't been run: they need the owner's OpenAI setup and approval.
The user guide is [Remote actors](../guide/cloud-assistants.md).

## 1. What it is

A remote actor, such as an OpenAI dot, reads what's going on on one
computer through a connection the owner pairs in XenoNet's window. It sees
the sessions in the projects the owner picked (statuses, times, counts,
attention items, other actors' relations there), and nothing else. It
changes nothing.

The path a read takes:

```
cloud assistant (a dot)
  -> OpenAI-managed tunnel endpoint
  -> tunnel-client, on the owner's computer
  -> xenonet mcp assistant --grant <grant>      mcp/assistant.js, stdio
  -> private endpoint, 127.0.0.1               server/assistant-endpoint.js
  -> assistant.read                            core/assistant/service.js
  -> XenoNet's in-memory state                   core/assistant/source.js
```

The ordinary HTTP API, its Host and Origin rules, the component service
and both existing MCP servers are unchanged and unreachable from a
connection.

## 2. Decisions

| Question | Decision | Why |
| --- | --- | --- |
| Local transport for the adapter | A separate private endpoint, not a role on the component service | The component service's hello takes the Tools token first, which the adapter must never hold. A separate listener with its own fresh allowlist (`hello`, `assistant.read`) leaves the Desktop and Tools roles' code untouched, and works in XenoNet Dev, where there's no component service. It reuses the same framing (`core/components/rpc.js`, now with a per-connection line limit). |
| When it listens | Only while a grant could be used | No listener at all for an owner who never pairs anything. It stops when the last grant is revoked or expires and when access is turned off. |
| Where the credential lives | `<state>/assistant/credentials/<grant>.json`, written with `core/private-file.js` and checked for other readers | Windows Credential Manager or the macOS keychain would need a new dependency or a native helper. The private-file helper already protects the component service's descriptor and actors' token files, on every supported OS. A file another account can read is deleted and pairing fails. |
| Revocation without the service | Tombstones in `<state>/assistant/` (`disabled`, `revoked/<grant>`) | They're honored by the service on every read and every two seconds, by the adapter before it serves anything from its cache, and on startup, where a tombstone wins over a database restored from a copy. Older XenoNets don't know grants at all, so they can't serve one. |
| Who may do what | Pairing, widening a grant, rotating and turning access back on: the window only, with its key. Listing, revoking and turning access off: the window or the owner's own terminal (`verify.person`) | Anything that creates or widens access needs the window. The fail-safe directions should work from a terminal, without the window, so an owner can stop access quickly. On Windows `verify.person` doesn't check processes yet, so the window key is what protects pairing there. |
| Audience | The owner says who can call the endpoint, and whether they checked. Until they say they checked, the grant reads made-up data only (`data_mode: fixture`) | The tunnel passes XenoNet no assertion of the caller, so XenoNet can't check the audience itself. Recording the owner's statement, and keeping real data off without it, follows the spec's rule that real data stays disabled until the audience is bounded. |
| Account links | Unverified, or the owner's word for an identity XenoNet already knows; never verified through this route | No transport here provides a provider assertion to validate. The verification path exists (`account-links.js`, `adapters.js`) and is tested with made-up evidence, so a later adapter with a real assertion plugs in without a new table. |
| Identity in snapshots | `identity` stays null for an owner-asserted link; `remote.accountLink` says how sure XenoNet is | Older clients show `identity` as fact, so a link that's only the owner's word never goes there. |
| Presence | A read is working for a 90-second lease, then unknown | A read is evidence of activity at that moment only. The lease ages to unknown, never offline, and the most present instance still wins, so an observer's word about Grok Bot outranks a lapsed lease. |
| Pagination | Keyset tokens, HMAC-signed with a key made at start, bound to grant, revision, tool and filters, five minutes | No server-side snapshot to keep or expire. The view is documented as the current one: an item that changes between pages can move. |
| Status of a session | From the tailer's signals: waiting (question or permission), stuck, working (a turn under way), completed (a turn ended and hasn't been looked at), idle, unknown (its source unavailable) | These are the signals the console already shows, so the answer matches what the owner sees. `status_basis` says which signal it came from. |
| Version | Not chosen | Release numbering and publication are the maintainer's. See [section 9](#9-release). |

## 3. The actor model

A connection belongs to an actor in the existing `actors` table; there is
no second registry. Migration 5 of `engine.sqlite` adds child tables:

- `actor_connections`: one per way an actor reaches XenoNet (transport,
  label, audience and the owner's word on it), cascading with its actor.
- `actor_grants`: what a connection may read (projects by their own ids,
  labels, lookback, data mode), until when, its revision, and its
  credential's SHA-256 only.
- `actor_account_links`, `actor_external_refs`: how sure XenoNet is of the
  account, and the provider's own ids once proven, unique per namespace;
  a conflict is refused for the owner to review, never merged.
- `assistant_places` (a place's `prj_<uuid>`, since node ids hold paths),
  `assistant_meta` (the device's id and a fingerprint of the installation),
  `assistant_audit` (seven days, 20,000 records).

Foreign keys are now on for `engine.sqlite`; the older tables declare
none. The legacy `actors.token_hash` is untouched. Remote-only actors have
`via: ["remote"]`, no `allowedAt`, no capabilities and no heartbeat, so
they don't count as registered, don't show an expired token, aren't
message-reachable, and don't take one of the 16 registered places.
`unregister` keeps an actor that still has remote connections. Removing an
actor revokes its grants first (`Actors.beforeRemove`), then the rows go
with it. `addVia`/`dropVia` change how XenoNet knows an actor without
touching its name, capabilities or identity; `upsert`'s replacement
semantics stay as they were for its existing callers.

Products gain `remote` beside `registers`: dots, Grok Bot and Hermes
Agent can be paired; none of them registers because of it. A new dot is
`actor:dots`, then `actor:dots-<4 hex>`; `productOfSlug` now reserves
`dots-…` for dots, as it does `hermes-…` for Hermes.

## 4. The trust boundary

What the owner approves is a grant, and what holds the grant is a local
credential readable only by the owner's account. The endpoint accepts a
hello only with the exact keys `role`, `api`, `grant_id` and `token`; a
wrong credential is refused like an unknown grant, and the right one hears
how its grant stands (revoked, expired, off). After hello, only
`assistant.read` is answered. The service, not the adapter, checks the
grant on each read and again just before the answer goes: revoked,
expired or off by then, the answer is dropped; changed in scope, the read
runs again once under the new revision.

What completes the boundary is outside XenoNet: the tunnel's configuration
and its audience, which the owner sets up with OpenAI and attests to.
Credential possession says nothing about which remote model is calling.

A state folder copied to another machine or account doesn't match the
fingerprint of the installation that made its grants, so they're revoked
(`device-changed`) and the device gets a new id.

## 5. Data policy

Every answer is made afresh from named fields (`core/assistant/
projection.js`) and checked against strict schemas (`core/schema/
assistant.js`, registered as `assistant/result/v1`, `assistant/error/v1`
and `actor/connection/v1`) before it goes. These are the only strict
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
| A remote caller with no grant | The tunnel's own access, then the endpoint's hello; a wrong credential hears nothing more than an unknown one | `assistant-endpoint.test.js` |
| A caller asking for an unselected project or session | Authorized view before any filter or count; uniform `NOT_FOUND` | `assistant-service.test.js` |
| A compromised adapter asking for more | Fresh method allowlist on the endpoint; every check in the service; the component service refuses the assistant role, and the endpoint refuses Tools tokens | `assistant-endpoint.test.js` |
| Prompt injection or secrets in titles | Labels off by default; cleaning when on; tool descriptions and instructions say labels are data, never instructions | `assistant-service.test.js` (canaries) |
| Stale authorization, replay | Recheck before release; revision-bound cursors and cache; tombstones; credential deleted on revoke; owner removal of a registered actor revokes its connections | `assistant-service.test.js`, `assistant-endpoint.test.js`, `assistant-connections.test.js` |
| A schema change adding fields | Strict output schemas checked before release | `assistant-service.test.js` |
| Overload | 16 KiB arguments, 256 KiB answers, 60 a minute with bursts of ten, four at a time, ten seconds, 320 KiB lines, refused hellos throttled | `assistant-service.test.js`, `assistant-endpoint.test.js` |
| A copied state folder | Installation fingerprint; grants revoked on mismatch | `assistant-connections.test.js` |
| Cached data after revocation, expiry or a narrower scope | The adapter serves a cached answer only while the grant's files say it stands (credential, tombstones, expiry) and only under the revision on disk now | `assistant-endpoint.test.js` |
| A stale endpoint file after a crash, naming a port someone else took | The adapter connects only while the process that wrote the file runs as this account | `assistant-endpoint.test.js` |
| Work piling up past the concurrency limit | A read that timed out keeps its slot until its work ends | `assistant-service.test.js` |

What this can't stop: code running as the owner's account can read the
credential file, the database and the transcripts themselves, and
change the tombstones. The design keeps a remote caller out of that
category; it doesn't claim to protect against a compromised account.

## 8. Checks

| Check | Result |
| --- | --- |
| G0a: adapter initialize, tools/list, tools/call locally (`--fixture`, and against a real grant) | Passed, Windows 11, Node 24 |
| G0b: the tunnel calls the fixture adapter | Not run: needs the owner's tunnel, credentials and approval |
| G0c: the actual dot lists the six tools and calls `xenonet_status` | Not run |
| G0d: the dot sees a fixture change with new times | Not run; the fixture changes every two minutes for it |
| G0e: a refused request, and local revocation stopping reads | Passed locally (endpoint, adapter, e2e); not through the tunnel |
| G0f: the endpoint's audience bounded and verified | Not run: the owner's to check in OpenAI's settings |
| T1–T6 (contract, authorization, minimization, failures, protocol, regression) | Covered by `test/assistant-*.test.js`, `test/engine-store.test.js` and the existing suites |
| T7, T8 (real dot, real data) | Not run |
| T9 platforms | Windows only. macOS and Linux are untested: treat them as experimental |
| End to end in XenoNet Dev's test instance | `test/e2e/assistant.test.js` passes |

Docs must not claim dot support until G0c and T7 pass on the actual
surface. Record those as their own outcomes; a test in local Codex,
ordinary ChatGPT, an MCP inspector or the Responses API doesn't establish
a dot's compatibility.

## 9. Release

Nothing here bumps a version or publishes anything. The change adds an
interface (assistant API 1), a data migration (engine 5, additive, with
its copy beside it) and opt-in UI; it fits a minor release under the
current policy. Release notes should say that remote connections are
new, off until paired, experimental, Windows-tested, and unconfirmed with
a real dot.

Rollback: turn access off (`xenonet assistant disable`), revoke the tunnel
with OpenAI, then select the previous Service and Tools versions. The
additive tables stay; an older XenoNet leaves the newer database's shape
alone, can't serve grants, and the tombstones keep revoked grants revoked
if a newer one is installed again.

## 10. Not in this version

Transcript access, a hosted gateway, aggregating several computers, and
any control (jobs, messages, approvals) are separate decisions (spec
sections 15 and 16). The device id is in every answer so a gateway can
aggregate later without a new contract.
