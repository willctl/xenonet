# Formal models

Quint models of the protocols where a bug costs most ([goal 9](../docs/design/goals.md), the [verification plan](../docs/design/direction.md#verification-plan)). There is one so far, `delivery.qnt`: how a receiver takes the entries of other machines' feeds.

```bash
npm run formal                               # typecheck, scenarios, random runs, seeded bugs
npm run formal -- --samples 100000 --steps 70  # more runs, longer ones
npm run formal -- --seed 0x6c7445f854ee425f    # reproduce one run of the design
```

`scripts/formal.mjs` runs Quint through `npx` at a pinned version (`@informalsystems/quint@0.33.1`), so it isn't a package dependency. The first run fetches Quint and its Rust evaluator (into `~/.quint`). The `formal` job in `.github/workflows/ci.yml` runs the same command.

## What `delivery.qnt` models

The receiver is `Feed::offer` ([crates/xenonet-log/src/feed.rs](../crates/xenonet-log/src/feed.rs)) as phase 5 extends it: every kept entry names the id of the entry before it, a compact entry stands for the range before it, and a revoked proof is refused by store order. Those three are ahead of the code (revisions 11d, 12d and 15 of the phase 5 plan); the model is the spec the code will follow.

- **Authors** write kept entries (open or close an item), loose entries (messages) and compact entries. They restore to a new incarnation that links at the last entry they still hold, and a faulty author equivocates: it signs a second entry at a place it already used and carries on from it.
- **Receivers** (R1, R2) hold entries behind a gap and send a Want, drop duplicates and stale incarnations, freeze an author on a fork, and keep loose dedupe records for the retention period.
- **The mailbox** keeps entries for offline receivers.
- **The honest holder** (H, a receiver too) serves its applied entries after the receiver's ack, on one ordered stream, when a receiver reconnects.
- **The adversary** is a member acting as holder or mailbox. It can't forge (entries are signed), but it serves any signed entry to anyone at any time: withholding, replays, subsets, stale incarnations, and the two sides of a fork to different receivers. The mailbox also replays, withholds and evicts. An iroh relay sees only QUIC ciphertext, so it has no part apart from the network.
- **The network** drops, duplicates, delays and reorders.

Bounds: two authors, three receivers, two items, two proofs, four kept entries per incarnation, three incarnations, two loose entries per author, six ticks of time, retention two ticks.

## Invariants

Checked in every state of every run (`safety` is their conjunction):

| # | Name | Says |
|---|---|---|
| 1 | `atMostOnce` | No entry is applied twice, kept or loose, whoever replays it. |
| 2 | `perAuthorOrder` | Each author's kept entries are applied in order of place. |
| 3 | `gapDetected` | Every applied kept entry follows the one before it: the next place naming its id, a later incarnation linking at or below it, or a compact entry covering what's between. While anything is held behind a gap, a Want is out. |
| 4 | `forkDetected` | A receiver with fork evidence in hand has frozen the author: an applied entry and an entry it held or applied at one place with different ids, one naming the other's place as its predecessor by another id, or an entry inside a compact entry's range and off its chain. This holds when the two sides came from two holders. |
| 5 | `compactEnds` | A receiver whose latest entry from an author is a compact entry has open exactly what that entry lists, so an item cancelled and compacted away is ended, not left running. |
| 6 | `revokedRefused` | Every entry stored after the receiver stored a revocation of its proof is refused, whenever the entry was signed. |
| 7 | `storedAfterReconnect` | Eventual storage, bounded (below). |

An invariant can pass because the runs never reach the states it's about, so `npm run formal` also fails unless some run reaches each witness: a frozen author, an entry held behind a gap, a compact entry applied across a gap, a restored incarnation linked, a refused entry, and a holder that served what it queued.

**Eventual storage as a bounded invariant.** On a reconnect the honest holder queues its applied kept entries after the receiver's ack, k of them (at most what it holds, here at most 24). Once it has served all k, the receiver stands at or past where the holder stood when it reconnected, or has frozen the author on a fork. The bound is sound because the stream is ordered and reliable while the connection is up, and because the holder's applied entries form a chain the receiver can follow from its own ack. Anything else may happen between the k steps: the adversary's entries, other deliveries, compaction. What the model can't say is that a connection stays up long enough, that a Want reaches some honest holder, or that a holder that answers "don't have" is skipped after 10 s. Those need fairness, which `quint run` doesn't check, and are left to the turmoil seeds (commit 8 on).

## Scenarios

`quint test` runs the plan's gates as fixed runs: a fork split across two holders (`splitForkTest`), cancel then compact (`cancelThenCompactTest`), a held entry behind a gap that a compact entry covers and a replay of it (`heldBehindCompactTest`), a fork inside a compacted range (`forkInCompactedRangeTest`), an entry signed before a revocation and delivered after it (`revokedWithheldTest`), a gap held and filled (`gapFilledTest`), and a restore below what a receiver applied (`restoreTest`).

## Seeded bugs

Each module at the end of `delivery.qnt` breaks one rule. `npm run formal` fails unless random runs find each one through the invariant named:

| Module | Broken rule | Caught by |
|---|---|---|
| `bugSkipGap` | takes a kept entry past a hole in its incarnation | `gapDetected` |
| `bugNoPrevCheck` | ignores the predecessor id an entry names | `forkDetected`, `gapDetected` |
| `bugNoCompactEnd` | keeps open what a compact entry no longer lists | `compactEnds` |
| `bugTimeRevocation` | refuses by the entry's signed time, not store order | `revokedRefused` |
| `bugPruneNoRefuse` | prunes loose dedupe records without refusing older entries | `atMostOnce` |

## Timings

On a 16-core M3 Max, `npm run formal` takes about 25 s: typecheck 4 s, scenarios 2 s, 20,000 runs of 50 steps of the design 7 to 13 s, and each seeded bug found in 1 to 3 s. 200,000 runs of 70 steps take about 100 s. The runs spread over the cores: held to four threads, a CI runner's count, the design's 20,000 runs take 16 s here, so the job should finish in a minute or two against its five-minute limit.

## What isn't modelled

- Signatures, hashes and sealing: an id stands for a hash, and nothing unsigned is ever delivered. Codecs are fuzzed instead.
- The envelope head, admission and membership: every member may serve every entry. Holders serving only members, and only the live set before the requester's admission, are revision 11b, for the network tests.
- Want routing, timeouts and rate limits (revision 11a, c), and mailbox quotas and Held receipts (revision 12a, b).
- Presence, HLC and clock skew (revisions 13, 14), and `any` arbitration (revision 16).
- Intents staying in `live` until their `until` passes (revision 15b): every item is a claim the author may end alone.
- A compact entry is checked against the header chain of the range it covers (the ids and predecessors of the entries in it). The model takes that chain from the set of signed entries, so commit 17 has to send it with a compact entry to a receiver that holds or applied entries in the range, or a fork there goes unseen.
- Receivers keep the id of every kept entry they applied (`FeedStore::kept`), which the fork rule needs; compaction on the receiver's side isn't modelled.
- Liveness and fairness (above).

## Why there's no `quint verify`

`quint verify` hands the model to Apalache for a bounded check of every behaviour, not a sample. It isn't run, nightly or otherwise, for two reasons. Apalache 0.62.4, the version Quint 0.33.1 fetches, needs Java 21; this Mac has Java 17, so it wasn't tried here, and CI would need a JDK step. And the model isn't written for Apalache. `restore` picks from an integer range whose bound depends on the state, where Apalache expects constant bounds; that part is a small change. The larger cost is the state: nested maps of records holding sets and lists, searched with folds over the signed entries, the shape an SMT encoding handles worst. A check at a useful depth would most likely need a smaller model written for Apalache. Until then the random runs and the scenarios are what checks the spec.
