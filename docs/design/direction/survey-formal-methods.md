# Formal methods options for XenoNet (research subagent, 2026-11-08)

## Tool verdicts (status checked 2026-11-08)
- TLA+/TLC: v1.8.0 pre-release 2026-11-06; Java 11+; Toolbox unmaintained, VS Code ext + CLI. Hard notation. Bridge to Rust = trace validation. Fits protocol + engine state machines.
- Apalache: apalache-mc org, v0.62.4 2026-10-12, maintained again; Java 21; bounded (--max-steps); good for inductive invariants.
- Quint (Informal): v0.33.1 2026-10-28; TS-like syntax, types, npm; `quint verify` via Apalache default or `--backend=tlc`; Rust evaluator for `quint run`. DECISIVE: quint-connect Rust crate (0.1.2, 2026-05-25) = model-based testing: generates traces from spec, replays against Rust driver, compares state each step, QUINT_SEED repro. Fits engine, authz, and escape hatch to TLA+.
- P (AWS): .NET 8 + Java; no Rust path; heavy. Not recommended.
- Alloy 6: 6.2.0 2025-01; relational + temporal; excellent for STRUCTURAL questions (many-to-many schema, authz matrix); weak for queues. Design tool, no Rust bridge.
- FizzBee: v0.5.4 2026-10; MBT Go only, Rust planned; immature. Watch.
- Stateright: Rust actor model checker; last release 0.31.0 2025-07-27, idle 15 months, single maintainer; wants Actor trait. Only if willing to fork.
- Kani (AWS): 0.68.0 2026-09-16; bounded MIR→CBMC; proves no panic/overflow/UB + assertions up to unwind bound; contracts; CI at Firecracker, s2n-quic, Rust std (16k harnesses). Fits lenient parsers, frame codec, ticket arithmetic, roster verify(). Not for distributed state machine.
- Verus: rolling 2026-11-05; Rust subset, ghost code; Amazon uses; AdaCore+CMU collab 2026-11-03; MS PDD kit. Too heavy today; revisit for pure roster-chain/CRDT merge.
- Creusot (v0.14.0 2026-10-12, opam/Why3), Prusti (stale since 2024), Aeneas (Lean/Coq): not now; Creusot for a pure CRDT merge later.
- Loom: v0.7.3 2026-11-08; only for hand-rolled lock-free. Skip.
- Shuttle (AWS): v0.9.10 2026-11-05; randomized reproducible scheduler; fits in-process engine concurrency if thread/lock based.
- Turmoil (tokio-rs): v0.7.2, pushed 2026-11-05; deterministic multi-host sim, turmoil-net/-fs, fault injection, seeds; AWS names shuttle+turmoil in CACM. Fits outbox/reconnect/ticket expiry under skew/wake-on-reply. Cost: route tokio::net via trait or cfg(turmoil).
- madsim: slower-moving, replaces runtime; prefer turmoil.
- DST/Antithesis: Antithesis = paid DST (Rust SDK, $105M Series A late 2025); turmoil gives 80% free.
- proptest (+ proptest-state-machine 0.9.0 2026-10-10), cargo-fuzz, bolero: zero cost. Fits parsers, roster/ticket state-machine properties, codec round trips.
- Tamarin 1.12.0 (2026-03), ProVerif 2.05 (2023), Verifpal 1.6.7 (2026-10; 1.0 Aug 2026; bounded 16 sessions, attacks re-executable). Kobeissi ePrint 2026/2683: all are "analyzers not provers". Verifpal = one afternoon; fits join handshake questions only; not CI.
- hax (Cryspen): skip (XenoNet uses library primitives; ProVerif backend experimental; "Verification Facade" critique 2026-04).
- Session types/typestate: skip libraries (Ferrite, rumpsteak, statum); DO hand-written typestate: Ticket<Fresh>→redeem(); Frame<Unverified>→Frame<Verified>.

## Model↔code sync state of the art
Trace validation (Cirstea/Kuppe/Loillier/Merz SEFM 2024, arXiv 2404.16075): log spec-variable updates, TLC checks trace membership. CCF in CI: 15 log statements, Trace spec, <1 s DFS; consensus trace spec ~2 engineer-months, consistency ~1 week; caught real bug (arXiv 2406.17455). MongoDB caution (VLDB 2020): trace-checking whole server vs abstract spec impractical; test generation from spec worked. Specula 2026 (arXiv 2607.25333): agents derive specs + trace validation. AWS PObserve. Lesson: spec at granularity of cheaply logged events; run conformance every CI run. Rust bridges: quint-connect (generation direction) and TLA+ trace validation (CCF direction).

## Recommended three-layer plan
A. Quint specs + quint-connect drivers: roster.qnt (hosts, gateway as adversary that relays/drops/reorders/duplicates/invents frames without keys, roster chain, tickets, outboxes; invariants: no two hosts accept conflicting roster views at same height; every roster entry traces to a member-signed frame; ticket used at most once; ticket rejected after 10 min; leadsTo: queued message delivered ≥1 after reconnect) and coord.qnt (claims never held by two; reply wakes waiter; delegated task exactly one owner answer). CI: quint run smoke 10 s; quint verify (Apalache, Java 21) --max-steps 10-15 for 3 hosts/2 tickets/2 claims, 1-2 min; #[quint_test] drivers in crates/xenonet-net/tests and engine tests replay hundreds of traces per PR. Keep models to variables drivers can observe.
B. turmoil: tokio::net behind cfg(turmoil) in gateway + host; seeded scenarios (gateway dies mid-join, reconnect after N drops, skew around ticket expiry, concurrent joins); 500-2000 seeds per PR, 100k nightly; fail with seed.
C. Leaf checks: cargo-fuzz + proptest on every lenient parser and frame codec; 2-3 Kani harnesses (frame decode never panics up to max size; ticket expiry arithmetic no overflow; roster verify() accepts only properly signed chains); typestate for one-shot resources.
One-off: Verifpal model of join (ticket, Ed25519, ChaCha20-Poly1305): authentication, confidentiality under passive-then-active gateway, ticket replay, freshness after host-key compromise. Rerun only when handshake changes.
Sync habits: protocol change edits spec first, PR links counterexample/new property; drivers next to Rust types; specs/scenarios/harnesses under crates/*/formal/ and docs/guide/formal.md names each property + tool. Never trace-check the whole service against one spec.
CI budget <5 min per PR in one `formal` job; nightly larger bounds/TLC liveness/100k seeds/long fuzz/15-min Kani.

## Do NOT verify formally
Authz decisions (encode matrix once, enumerate exhaustively in unit test; Alloy only if non-tabular after redesign); lenient parsers/drift (fuzz + golden files); UI/MCP plumbing/hooks (e2e); crypto primitives (trust audited crates); persistence/migrations (round-trip proptests); CRDT (pick published CRDT with proof or plain append-only log with total order; Creusot/Kani on one pure merge fn if hand-written).

Sources: see agent report (tlaplus, apalache-mc, informalsystems/quint, crates.io quint-connect, p-org, alloytools, fizzbee, stateright, model-checking/kani, verus-lang, creusot-rs, tokio-rs/loom, awslabs/shuttle, tokio-rs/turmoil, madsim, tigerbeetle, antithesis, proptest-state-machine, tamarin, proverif, symbolic.software verifpal + hax posts, eprint 2026/2683, cryspen/hax, arXiv 2404.16075, 2406.17455, 2006.00915, 2607.25333).
