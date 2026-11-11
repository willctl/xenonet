// Feed delivery (docs/design/direction.md, verification plan row 2).
//
// What a receiver does with the entries of an author's feed
// (crates/xenonet-log/src/feed.rs), as phase 5 extends it: kept entries
// name the id of the entry before them (revision 11d), a compact entry
// stands for the range before it (revision 15), and a revoked proof is
// refused by store order. The receivers here run that rule; everything
// around them may misbehave.
//
// Actors:
// - authors write kept entries (open or close an item), loose entries
//   (messages), compact entries, restore to a new incarnation, and may
//   equivocate (sign a second entry at a place they already used: a cloned
//   or badly restored machine);
// - receivers take entries through `offer`, the model of `Feed::offer`;
// - a mailbox keeps entries for receivers that are offline;
// - holders serve what they hold. H is an honest member that serves its
//   applied entries on reconnect. The adversary is a member acting as a
//   holder or as the mailbox: it withholds, replays, serves subsets, serves
//   stale incarnations and both sides of a fork (`inject` and the mailbox's
//   moves). It can't forge, since entries are signed: it sends only
//   entries some author signed. An iroh relay sees only QUIC ciphertext,
//   so it isn't modelled apart from the network.
// - the network drops, duplicates, delays and reorders.
//
// `BUG` seeds one broken rule (see the modules at the end); "none" is the
// design.

module deliveryCore {
  const BUG: str

  // ---- Bounds --------------------------------------------------------

  pure val AUTHORS = Set("A", "B")
  // H is a receiver too: the honest holder serves what it applied.
  pure val RECEIVERS = Set("R1", "R2", "H")
  pure val HOLDER = "H"
  pure val ITEMS = Set(1, 2)
  pure val PROOFS = Set("p1", "p2")
  // Kept entries per incarnation, compact entries included.
  pure val MAX_SEQ = 4
  pure val MAX_INC = 3
  pure val MAX_LOOSE = 2
  pure val MAX_TIME = 6
  // Loose entries older than this are refused, and their dedupe records
  // may be pruned (revision 12d).
  pure val RETENTION = 2
  // Longer than any chain: MAX_INC * MAX_SEQ entries plus the origin.
  pure val DEPTH = 14

  // ---- Data ----------------------------------------------------------

  type Pos = { inc: int, seq: int }
  pure val ORIGIN: Pos = { inc: 0, seq: 0 }
  pure def lt(p: Pos, q: Pos): bool = p.inc < q.inc or (p.inc == q.inc and p.seq < q.seq)
  pure def le(p: Pos, q: Pos): bool = p == q or lt(p, q)

  // One signed entry. `id` stands for the hash: two entries with the same
  // place and different ids are a fork. Kept entries name `prev`, the
  // place before them (the last entry of an earlier incarnation, for the
  // first of one), and `prevId`, that entry's id (0 for the origin).
  // `base`/`baseId` name the link of the entry's incarnation, which a
  // compact entry carries for a receiver that is behind it. `openAfter` is
  // what the author has open after writing it: the compact entry's
  // `live`, and what the invariants compare against.
  type Entry = {
    id: int,
    author: str,
    cls: str,
    inc: int,
    seq: int,
    prev: Pos,
    prevId: int,
    kind: str,
    item: int,
    live: Set[int],
    upto: int,
    base: Pos,
    baseId: int,
    proof: str,
    at: int,
    openAfter: Set[int],
  }

  pure def pos(e: Entry): Pos = { inc: e.inc, seq: e.seq }

  pure val NONE: Entry = {
    id: 0, author: "", cls: "", inc: 0, seq: 0, prev: ORIGIN, prevId: 0,
    kind: "", item: 0, live: Set(), upto: 0, base: ORIGIN, baseId: 0,
    proof: "", at: 0, openAfter: Set(),
  }

  type Author = {
    inc: int,
    // Kept entries written in this incarnation.
    seq: int,
    // The last kept entry before this incarnation (its first names it).
    link: Pos,
    linkId: int,
    // What the author still holds: every kept entry it can resend. A
    // compact entry replaces what it covers.
    store: List[Entry],
    open: Set[int],
    loose: int,
  }

  // What a receiver keeps of one author's feed.
  type Recv = {
    latest: Pos,
    latestId: int,
    kept: Pos -> Entry,
    held: Set[Entry],
    frozen: bool,
    open: Set[int],
    // A Want is out for what's after `latest`.
    want: bool,
  }

  pure val FRESH: Recv = {
    latest: ORIGIN, latestId: 0, kept: Map(), held: Set(), frozen: false,
    open: Set(), want: false,
  }

  // One entry entering the receiver's store, in store order.
  type Applied = { e: Entry, idx: int, refused: bool }

  type RState = {
    feeds: str -> Recv,
    // Every entry applied or taken, in store order.
    log: List[Applied],
    // The receiver's store order: entries and chain entries alike.
    idx: int,
    // Loose dedupe records.
    loose: Set[{ id: int, at: int }],
    // The revocations its chain holds, at their place in store order.
    revoked: str -> int,
    // Every kept entry it held or applied (for the fork invariant).
    had: Set[Entry],
  }

  type Msg = { to: str, e: Entry }

  // A reconnect to the honest holder: what it serves (its applied kept
  // entries after the receiver's ack, in order, on one reliable stream)
  // and where it stood when the receiver reconnected.
  type Session = { up: bool, queue: List[Entry], target: str -> Pos }

  type State = {
    now: int,
    nextId: int,
    signed: int -> Entry,
    authors: str -> Author,
    rs: str -> RState,
    net: Set[Msg],
    mailbox: Set[Msg],
    online: Set[str],
    session: str -> Session,
    revokedAt: str -> int,
    // The last action taken, so a trace reads as a story.
    last: str,
  }

  var s: State

  // ---- Chains --------------------------------------------------------

  // The id of the entry on `y`'s chain (y included) at place `p`, or -1
  // if the chain passes `p` by (it crossed into an earlier incarnation
  // below it). What a receiver checks a compact entry with: the compact
  // entry comes with the header chain of the range it covers.
  pure def ancestorAt(sg: int -> Entry, y: Entry, p: Pos): int =
    range(0, DEPTH).foldl({ cur: y.id, found: -1 }, (acc, _i) =>
      if (acc.found != -1 or acc.cur == 0) acc
      else
        val c = sg.get(acc.cur)
        if (pos(c) == p) { cur: 0, found: c.id }
        else if (lt(pos(c), p)) { cur: 0, found: -1 }
        else { cur: c.prevId, found: -1 }
    ).found

  // ---- The receiver --------------------------------------------------

  pure def isRevoked(st: RState, e: Entry, revokedAt: str -> int): bool =
    if (BUG == "timeRevocation")
      // Wrong: by the entry's signed time instead of store order.
      st.revoked.keys().contains(e.proof) and e.at >= revokedAt.get(e.proof)
    else
      st.revoked.keys().contains(e.proof)

  // The id the header chain of a compact entry the receiver applied puts
  // at `e`'s place, which the receiver skipped when it applied that entry
  // across a gap, or -1. The receiver keeps those ids (`FeedStore::know`).
  pure def skippedId(sg: int -> Entry, f: Recv, e: Entry): int =
    f.kept.keys()
      .filter(p => f.kept.get(p).kind == "compact" and p.inc == e.inc and e.seq <= f.kept.get(p).upto)
      .fold(-1, (acc, p) => if (acc != -1) acc else ancestorAt(sg, f.kept.get(p), pos(e)))

  // What the receiver does with a kept entry: "dup", "fork", "stale",
  // "apply" or "hold" (a gap: hold it and send a Want).
  pure def classify(sg: int -> Entry, f: Recv, e: Entry): str =
    val at = pos(e)
    val heldThere = f.held.filter(h => pos(h) == at)
    val keptPrev = f.kept.keys().contains(e.prev)
    val keptBase = f.kept.keys().contains(e.base)
    val skipped = skippedId(sg, f, e)
    if (f.frozen) "frozen"
    else if (f.kept.keys().contains(at))
      if (f.kept.get(at).id == e.id) "dup" else "fork"
    else if (heldThere != Set())
      if (heldThere.exists(h => h.id == e.id)) "dup" else "fork"
    // A place a compact entry covered: the chain's id there tells a copy
    // of the entry it skipped from the other side of a fork.
    else if (skipped != -1)
      if (skipped == e.id) "dup" else "fork"
    // The entry names a different entry before it than the one applied
    // there: one side of a fork, whichever holder it came from.
    else if (BUG != "noPrevCheck" and keptPrev and f.kept.get(e.prev).id != e.prevId) "fork"
    else if (BUG != "noPrevCheck" and e.kind == "compact" and keptBase and f.kept.get(e.base).id != e.baseId) "fork"
    else if (e.inc < f.latest.inc) "stale"
    else if (e.inc == f.latest.inc and le(at, f.latest)) "stale"
    else if (e.inc == f.latest.inc and e.seq == f.latest.seq + 1) "apply"
    // Wrong: takes an entry past a hole in the sequence.
    else if (BUG == "skipGap" and e.inc == f.latest.inc and e.seq > f.latest.seq + 1) "apply"
    else if (e.inc > f.latest.inc and e.seq == 1 and le(e.prev, f.latest)) "apply"
    else if (e.kind == "compact" and (
      (e.inc == f.latest.inc) or (e.inc > f.latest.inc and le(e.base, f.latest))
    ))
      // A compact entry ends the gap before it, if what the receiver
      // has applied and holds in its range is on its chain.
      val onChain = (e.inc != f.latest.inc or ancestorAt(sg, e, f.latest) == f.latestId)
      val heldOnChain = f.held.forall(h =>
        h.inc != e.inc or h.seq > e.upto or ancestorAt(sg, e, pos(h)) == h.id)
      if (onChain and heldOnChain) "apply" else "fork"
    else "hold"

  pure def applyEntry(st: RState, e: Entry, revokedAt: str -> int): RState =
    val f = st.feeds.get(e.author)
    val refused = e.kind != "compact" and isRevoked(st, e, revokedAt)
    val open =
      if (e.kind == "compact")
        // Wrong: keeps what the compact entry no longer lists.
        if (BUG == "noCompactEnd") f.open.union(e.live) else e.live
      else if (refused) f.open
      else if (e.kind == "open") f.open.union(Set(e.item))
      else f.open.exclude(Set(e.item))
    // Held entries at or below it can no longer apply: an earlier
    // incarnation closed, or a range a compact entry covers.
    val held = f.held.filter(h => lt(pos(e), pos(h)) and h.inc >= e.inc)
    val f2 = { ...f, latest: pos(e), latestId: e.id, kept: f.kept.put(pos(e), e), held: held, open: open }
    { ...st,
      feeds: st.feeds.set(e.author, f2),
      log: st.log.append({ e: e, idx: st.idx, refused: refused }),
      idx: st.idx + 1,
    }

  // Applies what's held and now in order, lowest place first, or freezes
  // on the first held entry that turns out to be a fork.
  pure def drain(sg: int -> Entry, st: RState, a: str, revokedAt: str -> int): RState =
    range(0, DEPTH).foldl(st, (st1, _i) =>
      val f = st1.feeds.get(a)
      val ready = f.held.filter(h =>
        val c = classify(sg, { ...f, held: f.held.exclude(Set(h)) }, h)
        c == "apply" or c == "fork")
      if (f.frozen or ready == Set()) st1
      else
        val h = ready.filter(x => ready.forall(o => le(pos(x), pos(o)))).fold(NONE, (_acc, x) => x)
        val f2 = { ...f, held: f.held.exclude(Set(h)) }
        if (classify(sg, f2, h) == "fork")
          { ...st1, feeds: st1.feeds.set(a, { ...f, frozen: true }) }
        else
          applyEntry({ ...st1, feeds: st1.feeds.set(a, f2) }, h, revokedAt)
    )

  pure def withWant(st: RState, a: str): RState =
    val f = st.feeds.get(a)
    { ...st, feeds: st.feeds.set(a, { ...f, want: f.held != Set() and not(f.frozen) }) }

  pure def offerKept(sg: int -> Entry, st: RState, e: Entry, revokedAt: str -> int): RState =
    val f = st.feeds.get(e.author)
    val c = classify(sg, f, e)
    val had = st.had.union(Set(e))
    if (c == "fork")
      { ...st, had: had, feeds: st.feeds.set(e.author, { ...f, frozen: true }) }
    else if (c == "hold")
      withWant({ ...st, had: had, feeds: st.feeds.set(e.author, { ...f, held: f.held.union(Set(e)) }) }, e.author)
    else if (c == "apply")
      withWant(drain(sg, applyEntry({ ...st, had: had }, e, revokedAt), e.author, revokedAt), e.author)
    else st

  pure def offerLoose(st: RState, e: Entry, now: int, revokedAt: str -> int): RState =
    val tooOld = BUG != "pruneNoRefuse" and e.at < now - RETENTION
    if (st.feeds.get(e.author).frozen or tooOld) st
    else if (st.loose.exists(x => x.id == e.id)) st
    else { ...st,
      log: st.log.append({ e: e, idx: st.idx, refused: isRevoked(st, e, revokedAt) }),
      idx: st.idx + 1,
      loose: st.loose.union(Set({ id: e.id, at: e.at })),
    }

  // `Feed::offer`, for an entry whose signature checked.
  pure def offer(st: State, r: str, e: Entry): State =
    val after =
      if (e.cls == "loose") offerLoose(st.rs.get(r), e, st.now, st.revokedAt)
      else offerKept(st.signed, st.rs.get(r), e, st.revokedAt)
    { ...st, rs: st.rs.set(r, after) }

  // ---- Init ----------------------------------------------------------

  action init = s' = {
    now: 0,
    nextId: 1,
    signed: Map(),
    authors: AUTHORS.mapBy(_a => {
      inc: 1, seq: 0, link: ORIGIN, linkId: 0, store: List(), open: Set(), loose: 0,
    }),
    rs: RECEIVERS.mapBy(_r => {
      feeds: AUTHORS.mapBy(_a => FRESH), log: List(), idx: 0, loose: Set(),
      revoked: Map(), had: Set(),
    }),
    net: Set(),
    mailbox: Set(),
    online: RECEIVERS,
    session: RECEIVERS.mapBy(_r => { up: false, queue: List(), target: Map() }),
    revokedAt: Map(),
    last: "init",
  }

  // ---- Authors -------------------------------------------------------

  pure def headOf(au: Author): { p: Pos, id: int } =
    if (au.seq == 0) { p: au.link, id: au.linkId }
    else
      val l = au.store[au.store.length() - 1]
      { p: pos(l), id: l.id }

  pure def mkKept(au: Author, a: str, id: int, kind: str, it: int, proof: str, at: int): Entry =
    val h = headOf(au)
    val openAfter =
      if (kind == "open") au.open.union(Set(it))
      else if (kind == "close") au.open.exclude(Set(it))
      else au.open
    { ...NONE,
      id: id, author: a, cls: "kept", inc: au.inc, seq: au.seq + 1,
      prev: h.p, prevId: h.id, kind: kind, item: it, live: if (kind == "compact") au.open else Set(),
      upto: if (kind == "compact") au.seq else 0,
      base: au.link, baseId: au.linkId, proof: proof, at: at, openAfter: openAfter,
    }

  action writeBy(a: str, kind: str, it: int, proof: str): bool = {
    val au = s.authors.get(a)
    val e = mkKept(au, a, s.nextId, kind, it, proof, s.now)
    all {
      au.seq < MAX_SEQ,
      s' = { ...s,
        nextId: s.nextId + 1,
        signed: s.signed.put(e.id, e),
        authors: s.authors.set(a, { ...au, seq: au.seq + 1, store: au.store.append(e), open: e.openAfter }),
        last: "write",
      },
    }
  }

  action write = {
    nondet a = AUTHORS.oneOf()
    nondet kind = Set("open", "close").oneOf()
    nondet it = ITEMS.oneOf()
    nondet proof = PROOFS.oneOf()
    writeBy(a, kind, it, proof)
  }

  // Retention: a compact entry lists what is still open and replaces
  // everything the author held before it.
  action compactBy(a: str): bool = {
    val au = s.authors.get(a)
    val c = mkKept(au, a, s.nextId, "compact", 0, "none", s.now)
    all {
      au.seq >= 1,
      au.seq < MAX_SEQ,
      s' = { ...s,
        nextId: s.nextId + 1,
        signed: s.signed.put(c.id, c),
        authors: s.authors.set(a, { ...au, seq: au.seq + 1, store: List(c) }),
        last: "compact",
      },
    }
  }

  action compact = {
    nondet a = AUTHORS.oneOf()
    compactBy(a)
  }

  // The author's store was restored from a copy that holds only its
  // first `n` entries, or XenoNet was set up again: a new incarnation,
  // whose first entry names the last kept entry the author still holds.
  action restoreBy(a: str, n: int): bool = {
    val au = s.authors.get(a)
    val remaining = au.store.slice(0, n)
    val lastE = if (n == 0) NONE else remaining[n - 1]
    all {
      au.inc < MAX_INC,
      s' = { ...s,
        authors: s.authors.set(a, {
          inc: au.inc + 1, seq: 0,
          link: if (n == 0) ORIGIN else pos(lastE), linkId: lastE.id,
          store: remaining, open: lastE.openAfter, loose: au.loose,
        }),
        last: "restore",
      },
    }
  }

  action restore = {
    nondet a = AUTHORS.oneOf()
    nondet n = 0.to(s.authors.get(a).store.length()).oneOf()
    restoreBy(a, n)
  }

  // A faulty author signs a second entry at the place of its last one,
  // and goes on from it: a fork, which holders may serve split.
  action equivocateBy(a: str, kind: str, it: int): bool = {
    val au = s.authors.get(a)
    val n = au.store.length()
    val old = if (n == 0) NONE else au.store[n - 1]
    val parentOpen = if (old.prevId == 0) Set() else s.signed.get(old.prevId).openAfter
    val openAfter = if (kind == "open") parentOpen.union(Set(it)) else parentOpen.exclude(Set(it))
    val e = { ...old, id: s.nextId, kind: kind, item: it, openAfter: openAfter }
    all {
      au.seq >= 1,
      old.inc == au.inc,
      old.kind != "compact",
      s' = { ...s,
        nextId: s.nextId + 1,
        signed: s.signed.put(e.id, e),
        authors: s.authors.set(a, { ...au, store: au.store.replaceAt(n - 1, e), open: openAfter }),
        last: "equivocate",
      },
    }
  }

  action equivocate = {
    nondet a = AUTHORS.oneOf()
    nondet kind = Set("open", "close").oneOf()
    nondet it = ITEMS.oneOf()
    equivocateBy(a, kind, it)
  }

  action writeLoose = {
    nondet a = AUTHORS.oneOf()
    nondet proof = PROOFS.oneOf()
    val au = s.authors.get(a)
    val e = { ...NONE,
      id: s.nextId, author: a, cls: "loose", inc: au.inc, seq: au.loose + 1,
      kind: "msg", proof: proof, at: s.now,
    }
    all {
      au.loose < MAX_LOOSE,
      s' = { ...s,
        nextId: s.nextId + 1,
        signed: s.signed.put(e.id, e),
        authors: s.authors.set(a, { ...au, loose: au.loose + 1 }),
        last: "writeLoose",
      },
    }
  }

  // The author sends (or resends) an entry it holds, or one of its loose
  // entries: on the link if the receiver is online, to the mailbox if not.
  action send = {
    nondet a = AUTHORS.oneOf()
    nondet r = RECEIVERS.oneOf()
    val au = s.authors.get(a)
    val mine = au.store.foldl(Set(), (acc, x) => acc.union(Set(x)))
      .union(s.signed.keys().map(i => s.signed.get(i)).filter(x => x.author == a and x.cls == "loose"))
    all {
      mine != Set(),
      nondet e = mine.oneOf()
      val m = { to: r, e: e }
      s' = if (s.online.contains(r)) { ...s, net: s.net.union(Set(m)), last: "send" }
           else { ...s, mailbox: s.mailbox.union(Set(m)), last: "send to mailbox" },
    }
  }

  // ---- Network and mailbox -------------------------------------------

  // Delivered in any order; a copy may stay behind (a duplicate).
  action deliver = all {
    s.net != Set(),
    nondet m = s.net.oneOf()
    nondet dup = Set(true, false).oneOf()
    all {
      s.online.contains(m.to),
      val s1 = offer(s, m.to, m.e)
      s' = { ...s1, net: if (dup) s.net else s.net.exclude(Set(m)), last: "deliver" },
    },
  }

  action drop = all {
    s.net != Set(),
    nondet m = s.net.oneOf()
    s' = { ...s, net: s.net.exclude(Set(m)), last: "drop" },
  }

  // The mailbox hands over what it keeps, in any order, again if it likes
  // (a replay), or loses it (withholds, evicts).
  action mailboxDeliver = all {
    s.mailbox != Set(),
    nondet m = s.mailbox.oneOf()
    nondet keep = Set(true, false).oneOf()
    all {
      s.online.contains(m.to),
      val s1 = offer(s, m.to, m.e)
      s' = { ...s1, mailbox: if (keep) s.mailbox else s.mailbox.exclude(Set(m)), last: "mailbox deliver" },
    },
  }

  action mailboxDrop = all {
    s.mailbox != Set(),
    nondet m = s.mailbox.oneOf()
    s' = { ...s, mailbox: s.mailbox.exclude(Set(m)), last: "mailbox drop" },
  }

  // A member acting as a holder or mailbox serves any entry it has seen,
  // whenever it likes: old ones again, stale incarnations, one side of a
  // fork to one receiver and the other side to another, any subset.
  action injectTo(r: str, i: int): bool = all {
    s.online.contains(r),
    s.signed.keys().contains(i),
    s' = { ...offer(s, r, s.signed.get(i)), last: "inject" },
  }

  action inject = all {
    s.signed != Map(),
    nondet r = RECEIVERS.oneOf()
    nondet i = s.signed.keys().oneOf()
    injectTo(r, i),
  }

  action goOffline = {
    nondet r = RECEIVERS.oneOf()
    all {
      s.online.contains(r),
      s' = { ...s,
        online: s.online.exclude(Set(r)),
        // Its connections close, the honest holder's included.
        session: s.session.keys().mapBy(x =>
          if (x == r or r == HOLDER) { ...s.session.get(x), up: false, queue: List() }
          else s.session.get(x)),
        last: "offline",
      },
    }
  }

  action goOnline = {
    nondet r = RECEIVERS.oneOf()
    all {
      not(s.online.contains(r)),
      s' = { ...s, online: s.online.union(Set(r)), last: "online" },
    }
  }

  // ---- The honest holder ---------------------------------------------

  // A receiver reconnects to H, which serves its applied kept entries
  // after the receiver's ack on one ordered stream.
  action reconnectBy(r: str): bool = {
    val h = s.rs.get(HOLDER)
    val mine = s.rs.get(r)
    val queue = h.log.select(rec =>
      rec.e.cls == "kept" and lt(mine.feeds.get(rec.e.author).latest, pos(rec.e))).foldl(List(), (l, rec) => l.append(rec.e))
    all {
      s.online.contains(r),
      s.online.contains(HOLDER),
      not(s.session.get(r).up),
      s' = { ...s,
        session: s.session.set(r, { up: true, queue: queue, target: AUTHORS.mapBy(a => h.feeds.get(a).latest) }),
        last: "reconnect",
      },
    }
  }

  action reconnect = {
    nondet r = RECEIVERS.exclude(Set(HOLDER)).oneOf()
    reconnectBy(r)
  }

  action serveTo(r: str): bool = {
    val ses = s.session.get(r)
    all {
      ses.up,
      ses.queue.length() > 0,
      val s1 = offer(s, r, ses.queue.head())
      s' = { ...s1, session: s.session.set(r, { ...ses, queue: ses.queue.tail() }), last: "serve" },
    }
  }

  action serve = {
    nondet r = RECEIVERS.exclude(Set(HOLDER)).oneOf()
    serveTo(r)
  }

  action disconnect = {
    nondet r = RECEIVERS.exclude(Set(HOLDER)).oneOf()
    all {
      s.session.get(r).up,
      s' = { ...s, session: s.session.set(r, { ...s.session.get(r), up: false, queue: List() }), last: "disconnect" },
    }
  }

  // ---- Time, revocation, pruning -------------------------------------

  action tick = all {
    s.now < MAX_TIME,
    s' = { ...s, now: s.now + 1, last: "tick" },
  }

  // The person revokes a proof in the network chain.
  action revokeProof(p: str): bool = all {
    not(s.revokedAt.keys().contains(p)),
    s' = { ...s, revokedAt: s.revokedAt.put(p, s.now), last: "revoke" },
  }

  action revoke = {
    nondet p = PROOFS.oneOf()
    revokeProof(p)
  }

  // The chain entry reaches a receiver, which stores it in its order.
  action learnBy(r: str, p: str): bool = {
    val st = s.rs.get(r)
    all {
      s.revokedAt.keys().contains(p),
      not(st.revoked.keys().contains(p)),
      s' = { ...s, rs: s.rs.set(r, { ...st, revoked: st.revoked.put(p, st.idx), idx: st.idx + 1 }), last: "learn revocation" },
    }
  }

  action learnRevocation = {
    nondet r = RECEIVERS.oneOf()
    nondet p = PROOFS.oneOf()
    learnBy(r, p)
  }

  action prune = {
    nondet r = RECEIVERS.oneOf()
    val st = s.rs.get(r)
    s' = { ...s, rs: s.rs.set(r, { ...st, loose: st.loose.filter(x => x.at >= s.now - RETENTION) }), last: "prune" }
  }

  action step = any {
    write, compact, restore, equivocate, writeLoose, send,
    deliver, drop, mailboxDeliver, mailboxDrop, inject,
    goOffline, goOnline, reconnect, serve, disconnect,
    tick, revoke, learnRevocation, prune,
  }

  // ---- Invariants ----------------------------------------------------

  pure def keptOf(st: RState, a: str): List[Entry] =
    st.log.select(rec => rec.e.cls == "kept" and rec.e.author == a).foldl(List(), (l, rec) => l.append(rec.e))

  // 1. An entry is applied at most once.
  val atMostOnce = RECEIVERS.forall(r =>
    val l = s.rs.get(r).log
    l.indices().forall(i => l.indices().forall(j => i == j or l[i].e.id != l[j].e.id)))

  // 2. Kept entries are applied in per-author order.
  val perAuthorOrder = RECEIVERS.forall(r => AUTHORS.forall(a =>
    val l = keptOf(s.rs.get(r), a)
    l.indices().forall(i => i == 0 or lt(pos(l[i - 1]), pos(l[i])))))

  // How each applied kept entry follows the one before it: the next place
  // naming it, a later incarnation linking at or below it, or a compact
  // entry that covers the places between.
  pure def links(x: Entry, y: Entry): bool = or {
    y.inc == x.inc and y.seq == x.seq + 1 and y.prevId == x.id,
    y.inc > x.inc and y.seq == 1 and le(y.prev, pos(x)),
    y.kind == "compact" and y.inc == x.inc and x.seq < y.upto,
    y.kind == "compact" and y.inc > x.inc and le(y.base, pos(x)),
  }

  // 3. A gap is always detected, never silently skipped: every applied
  // kept entry links to the one before, and while anything is held behind
  // a gap, a Want is out.
  val gapDetected = RECEIVERS.forall(r => AUTHORS.forall(a =>
    val f = s.rs.get(r).feeds.get(a)
    val l = keptOf(s.rs.get(r), a)
    and {
      l.indices().forall(i => links(if (i == 0) NONE else l[i - 1], l[i])),
      f.held == Set() or f.frozen or f.want,
    }))

  // Evidence of a fork a receiver has in hand: an applied entry `x` and an
  // entry `y` it held or applied, at one place with different ids, or one
  // naming another as the entry before it by a different id, or `y` inside
  // a compact entry's range and off its chain.
  pure def evidence(sg: int -> Entry, x: Entry, y: Entry): bool = x.author == y.author and or {
    pos(x) == pos(y) and x.id != y.id,
    y.prev == pos(x) and y.prevId != x.id,
    x.prev == pos(y) and x.prevId != y.id,
    x.kind == "compact" and y.inc == x.inc and y.seq <= x.upto and ancestorAt(sg, x, pos(y)) != y.id,
  }

  // 4. A fork is detected even when it is split across two holders: a
  // receiver with evidence of one has frozen the author.
  val forkDetected = RECEIVERS.forall(r => AUTHORS.forall(a =>
    val st = s.rs.get(r)
    val f = st.feeds.get(a)
    f.frozen or f.kept.keys().forall(p => st.had.forall(y => not(evidence(s.signed, f.kept.get(p), y))))))

  // 5. After compaction a cancel is never lost: a receiver whose latest
  // entry from an author is a compact entry has open exactly what it lists,
  // so whatever it held open that the compact entry omits is ended.
  val compactEnds = RECEIVERS.forall(r => AUTHORS.forall(a =>
    val f = s.rs.get(r).feeds.get(a)
    f.latest == ORIGIN or f.kept.get(f.latest).kind != "compact" or f.open == f.kept.get(f.latest).live))

  // 6. A revoked proof is refused for every entry stored after the
  // revocation, whenever the entry was signed.
  val revokedRefused = RECEIVERS.forall(r =>
    val st = s.rs.get(r)
    st.log.indices().forall(i =>
      val rec = st.log[i]
      not(st.revoked.keys().contains(rec.e.proof) and st.revoked.get(rec.e.proof) < rec.idx) or rec.refused))

  // 7. Eventual storage, bounded: once the honest holder has served the k
  // entries it queued on a reconnect (k at most the entries it holds), the
  // receiver stands at least where the holder stood, unless it froze the
  // author on a fork.
  val storedAfterReconnect = RECEIVERS.forall(r =>
    val ses = s.session.get(r)
    not(ses.up and ses.queue.length() == 0) or AUTHORS.forall(a =>
      val f = s.rs.get(r).feeds.get(a)
      f.frozen or le(ses.target.get(a), f.latest)))

  val safety = and {
    atMostOnce, perAuthorOrder, gapDetected, forkDetected,
    compactEnds, revokedRefused, storedAfterReconnect,
  }

  // ---- Scenarios (quint test): the gates of revisions 11, 12 and 15 ----

  pure def feedOf(st: State, r: str, a: str): Recv = st.rs.get(r).feeds.get(a)
  pure def appliedIds(st: State, r: str): List[int] = st.rs.get(r).log.foldl(List(), (l, rec) => l.append(rec.e.id))

  // A forks at (1,2). The honest holder H has the first side and serves
  // it to R1 on reconnect; the mailbox serves R1 the entry after the
  // other side. R1 freezes A.
  run splitForkTest =
    init
      .then(writeBy("A", "open", 1, "p1"))      // 1 at (1,1)
      .then(writeBy("A", "open", 2, "p1"))      // 2 at (1,2)
      .then(injectTo("H", 1))
      .then(injectTo("H", 2))
      .then(equivocateBy("A", "close", 1))      // 3 at (1,2)
      .then(writeBy("A", "close", 2, "p1"))     // 4 at (1,3), after 3
      .then(reconnectBy("R1"))
      .then(serveTo("R1"))
      .then(serveTo("R1"))
      .then(injectTo("R1", 4))
      .expect(feedOf(s, "R1", "A").frozen and safety)

  // B started on A's open item, A cancelled it and compacted the cancel
  // away; B, back late, gets only the compact entry and ends the item.
  run cancelThenCompactTest =
    init
      .then(writeBy("A", "open", 1, "p1"))      // 1
      .then(injectTo("R1", 1))
      .then(writeBy("A", "close", 1, "p1"))     // 2
      .then(compactBy("A"))                     // 3, live {}
      .then(injectTo("R1", 3))
      .expect(feedOf(s, "R1", "A").open == Set() and feedOf(s, "R1", "A").latest == { inc: 1, seq: 3 } and safety)

  // A held entry behind a gap doesn't come back once a compact entry has
  // covered it, and its replay is stale.
  run heldBehindCompactTest =
    init
      .then(writeBy("A", "open", 1, "p1"))      // 1
      .then(writeBy("A", "open", 2, "p1"))      // 2
      .then(writeBy("A", "close", 2, "p1"))     // 3
      .then(compactBy("A"))                     // 4, live {1}
      .then(injectTo("R1", 1))
      .then(injectTo("R1", 3))
      .expect(feedOf(s, "R1", "A").want)
      .then(injectTo("R1", 4))
      .then(injectTo("R1", 2))
      .expect(and {
        feedOf(s, "R1", "A").open == Set(1),
        feedOf(s, "R1", "A").held == Set(),
        appliedIds(s, "R1") == [1, 4],
        safety,
      })

  // A fork inside the range a compact entry covers is still detected by
  // a receiver that holds the other side.
  run forkInCompactedRangeTest =
    init
      .then(writeBy("A", "open", 1, "p1"))      // 1
      .then(writeBy("A", "open", 2, "p1"))      // 2
      .then(writeBy("A", "close", 1, "p1"))     // 3 at (1,3)
      .then(equivocateBy("A", "close", 2))      // 4 at (1,3)
      .then(compactBy("A"))                     // 5, after 4
      .then(injectTo("R1", 1))
      .then(injectTo("R1", 3))
      .then(injectTo("R1", 5))
      .expect(feedOf(s, "R1", "A").frozen and safety)

  // A receiver that applied a compact entry across a gap, then gets the
  // other side of a fork at a place it skipped: the chain's id there
  // shows it, and a copy of the side on the chain is a duplicate.
  run forkAfterJumpTest =
    init
      .then(writeBy("A", "open", 1, "p1"))      // 1 at (1,1)
      .then(writeBy("A", "open", 2, "p1"))      // 2 at (1,2)
      .then(equivocateBy("A", "close", 1))      // 3 at (1,2)
      .then(writeBy("A", "close", 2, "p1"))     // 4 at (1,3), after 3
      .then(compactBy("A"))                     // 5, after 4
      .then(injectTo("R1", 1))
      .then(injectTo("R1", 5))
      .then(injectTo("R1", 3))
      .expect(not(feedOf(s, "R1", "A").frozen) and feedOf(s, "R1", "A").latest == { inc: 1, seq: 4 } and safety)
      .then(injectTo("R1", 2))
      .expect(feedOf(s, "R1", "A").frozen and safety)

  // Signed with p1 before the revocation, withheld by the mailbox, and
  // delivered after R1 stored the revocation: refused.
  run revokedWithheldTest =
    init
      .then(writeBy("A", "open", 1, "p1"))      // 1, at time 0
      .then(tick)
      .then(revokeProof("p1"))
      .then(learnBy("R1", "p1"))
      .then(injectTo("R1", 1))
      .expect(s.rs.get("R1").log[0].refused and feedOf(s, "R1", "A").open == Set() and safety)

  // A gap is held with a Want out, and fills in order.
  run gapFilledTest =
    init
      .then(writeBy("A", "open", 1, "p1"))      // 1
      .then(writeBy("A", "open", 2, "p1"))      // 2
      .then(injectTo("R1", 2))
      .expect(feedOf(s, "R1", "A").want and appliedIds(s, "R1") == [])
      .then(injectTo("R1", 1))
      .expect(not(feedOf(s, "R1", "A").want) and appliedIds(s, "R1") == [1, 2] and safety)

  // A restore to a copy holding two entries: the new incarnation links
  // below what R1 has applied, which stays applied. R2, which saw
  // nothing, takes all four in order whatever order they come in.
  run restoreTest =
    init
      .then(writeBy("A", "open", 1, "p1"))      // 1
      .then(writeBy("A", "open", 2, "p1"))      // 2
      .then(writeBy("A", "close", 1, "p1"))     // 3
      .then(injectTo("R1", 1))
      .then(injectTo("R1", 2))
      .then(injectTo("R1", 3))
      .then(restoreBy("A", 2))
      .then(writeBy("A", "close", 2, "p1"))     // 4 at (2,1), after 2
      .then(injectTo("R1", 4))
      .then(injectTo("R2", 4))
      .then(injectTo("R2", 3))
      .then(injectTo("R2", 1))
      .then(injectTo("R2", 2))
      .expect(and {
        feedOf(s, "R1", "A").latest == { inc: 2, seq: 1 },
        appliedIds(s, "R2") == [1, 2, 3, 4],
        safety,
      })

  // ---- Witnesses: states the runs must reach, or the invariants say little

  val someFrozen = RECEIVERS.exists(r => AUTHORS.exists(a => s.rs.get(r).feeds.get(a).frozen))
  val someRestoreLinked = RECEIVERS.exists(r => AUTHORS.exists(a => s.rs.get(r).feeds.get(a).latest.inc >= 2))
  val someRefused = RECEIVERS.exists(r => s.rs.get(r).log.indices().exists(i => s.rs.get(r).log[i].refused))
  val someHeld = RECEIVERS.exists(r => AUTHORS.exists(a => s.rs.get(r).feeds.get(a).want))
  val someJump = RECEIVERS.exists(r => AUTHORS.exists(a =>
    val l = keptOf(s.rs.get(r), a)
    l.indices().exists(i => l[i].kind == "compact" and (i == 0 or l[i - 1].seq < l[i].upto or l[i - 1].inc < l[i].inc))))
  val someServed = RECEIVERS.exists(r => s.session.get(r).up and s.session.get(r).queue.length() == 0 and
    AUTHORS.exists(a => s.session.get(r).target.get(a) != ORIGIN))
}

// The design: what `npm run formal` checks.
module delivery {
  import deliveryCore(BUG = "none").*
}

// Seeded bugs: each breaks one rule, and `quint run` must find the
// invariant it violates (npm run formal checks that it does).
module bugSkipGap {
  import deliveryCore(BUG = "skipGap").*
}

module bugNoPrevCheck {
  import deliveryCore(BUG = "noPrevCheck").*
}

module bugNoCompactEnd {
  import deliveryCore(BUG = "noCompactEnd").*
}

module bugTimeRevocation {
  import deliveryCore(BUG = "timeRevocation").*
}

module bugPruneNoRefuse {
  import deliveryCore(BUG = "pruneNoRefuse").*
}