// A space's chain (docs/design/direction.md, "Xenolith"; verification
// plan rows 1 and 3).
//
// What a holder of a space does with the entries it has
// (crates/xenonet-log/src/space.rs, `Space::rebuild` and `Space::level`):
// sequenced entries (the owner's shares, appended by the curator of their
// epoch) and side entries (the owner's unshares, takes and moves) give the
// effective chain by one rule, whatever order they came in. Each epoch's
// run grows from where it starts with entries its curator signed, and
// stops at a fork; of the takes and moves for the next epoch whose base is
// on the run, the lowest id wins and the run is cut after its base. A
// subject's level is its last share on the chain, unless the holder holds
// an unshare of it that no share of it lists as seen.
//
// Actors:
// - the owner, at any holder (one of their devices is there): shares
//   through the curator, unshares, takes, and moves with the curator;
// - holders A, B and C, each with its own copy. A is the founding curator.
//   A curator sequences only while its own view says it's the curator, and
//   may misbehave by signing two entries at one place (equivocate);
// - the adversary: any member may sign entries as itself (a share with no
//   owner's signature, a take that isn't the owner's) and serve any signed
//   entry to anyone at any time, or never: partitions, withholding,
//   replays and every order of arrival. It can't forge the owner.
//
// `BUG` seeds one broken rule (see the modules at the end); "none" is the
// design.

module spaceCore {
  const BUG: str

  // ---- Bounds --------------------------------------------------------

  pure val HOLDERS = Set("A", "B", "C")
  pure val FOUNDER = "A"
  pure val SUBJECTS = Set("S1", "S2")
  pure val LEVELS = Set(1, 2)
  // Entries signed in a run, entry 0 aside.
  pure val MAX_IDS = 12
  // Sequenced entries on a chain, entry 0 aside.
  pure val MAX_N = 4
  pure val MAX_EPOCH = 3

  // ---- Data ----------------------------------------------------------

  // One signed entry. `id` stands for the hash, and ranks takes. A
  // sequenced entry ("seq") is at `n` after `prev`, in `epoch`, signed by
  // its `curator` and, when `owner`, by the owner. A side entry is an
  // "unshare" of `subject`, or a "take" or "move" to `to` at `epoch` from
  // the chain entry `baseId` at `baseN`; a move is signed by the curator
  // it moves from (`curator`) too.
  type E = {
    id: int,
    cls: str,
    n: int,
    prev: int,
    epoch: int,
    curator: str,
    owner: bool,
    subject: str,
    level: int,
    after: Set[int],
    to: str,
    baseN: int,
    baseId: int,
  }

  pure val NONE: E = {
    id: 0, cls: "", n: 0, prev: 0, epoch: 0, curator: "", owner: false,
    subject: "", level: 0, after: Set(), to: "", baseN: 0, baseId: 0,
  }
  pure val GENESIS: E = { ...NONE, cls: "found", curator: FOUNDER, owner: true }

  // The effective chain as a holder builds it.
  type View = { path: List[int], epoch: int, curator: str, start: int, frozen: bool }

  pure val START: View = { path: [0], epoch: 0, curator: FOUNDER, start: 0, frozen: false }

  // What a holder has: the ids it holds, when it stored each in its own
  // order, the reads it served (a subject, at its store order then), and
  // the view `viewOf` gives of what it holds, kept so the invariants don't
  // build it again.
  type Holder = {
    known: Set[int],
    storedAt: int -> int,
    clock: int,
    served: Set[{ s: str, at: int }],
    view: View,
  }

  type State = {
    nextId: int,
    signed: int -> E,
    hs: str -> Holder,
    // Ghosts: the levels the owner gave each subject, and the curators the
    // owner named.
    granted: Set[{ s: str, l: int }],
    named: Set[str],
    last: str,
  }

  var s: State

  // ---- The holder's rule ---------------------------------------------

  pure def tip(v: View): int = v.path[v.path.length() - 1]

  // A sequenced entry counts in `v`'s epoch when that epoch's curator
  // and the owner signed it.
  pure def validSeq(e: E, v: View): bool = and {
    e.cls == "seq",
    e.epoch == v.epoch,
    e.curator == v.curator,
    BUG == "noOwnerCheck" or e.owner,
  }

  // The current epoch's run, from where it stands: the one entry that
  // follows the head, until none does or two do (a fork: frozen).
  pure def runOf(es: Set[E], v: View): View =
    range(0, MAX_N).foldl(v, (acc, _i) =>
      if (acc.frozen) acc
      else
        val c = es.filter(e => e.n == acc.path.length() and e.prev == tip(acc) and validSeq(e, acc))
        if (c.size() == 0) acc
        // Wrong: takes the lowest of a fork's sides and carries on.
        else if (c.size() > 1 and BUG != "noForkFreeze") { ...acc, frozen: true }
        else { ...acc, path: acc.path.append(c.fold(MAX_IDS + 1, (m, e) => if (e.id < m) e.id else m)) }
    )

  // A take or a move for the next epoch, whose base is on this epoch's
  // run, by the owner (and, a move, by the curator it moves from).
  pure def validT(t: E, v: View): bool = and {
    t.cls == "take" or t.cls == "move",
    t.epoch == v.epoch + 1,
    t.baseN >= v.start,
    t.baseN < v.path.length(),
    v.path[t.baseN] == t.baseId,
    t.to != v.curator,
    BUG == "noOwnerCheck" or t.owner,
    t.cls == "take" or t.curator == v.curator,
  }

  // The lowest id wins. Wrong: the one this holder stored first.
  pure def rank(h: Holder, t: E): int = if (BUG == "firstTakeWins") h.storedAt.get(t.id) else t.id

  pure def viewOf(sg: int -> E, h: Holder): View =
    val es = h.known.map(i => sg.get(i))
    val v = range(0, MAX_EPOCH + 1).foldl(START, (v0, _i) =>
      val r = runOf(es, v0)
      val ts = es.filter(t => validT(t, r))
      if (ts.size() == 0) r
      else
        val w = ts.fold(NONE, (b, t) => if (b.cls == "" or rank(h, t) < rank(h, b)) t else b)
        { path: r.path.slice(0, w.baseN + 1), epoch: r.epoch + 1, curator: w.to, start: w.baseN, frozen: false }
    )
    runOf(es, v)

  pure def unsharesOf(sg: int -> E, h: Holder, x: str): Set[int] =
    h.known.filter(i => sg.get(i).cls == "unshare" and sg.get(i).subject == x and (BUG == "noOwnerCheck" or sg.get(i).owner))

  pure def levelOf(sg: int -> E, h: Holder, x: str): int =
    val v = h.view
    val shares = v.path.foldl(List(), (l, i) => if (sg.get(i).cls == "seq" and sg.get(i).subject == x) l.append(sg.get(i)) else l)
    val seen = shares.foldl(Set(), (acc, e) => acc.union(e.after))
    if (shares.length() == 0) 0
    // Wrong: serves by the last share, whatever unshare it holds.
    else if (BUG != "serveAfterUnshare" and unsharesOf(sg, h, x).exists(u => not(seen.contains(u)))) 0
    else shares[shares.length() - 1].level

  // ---- Init ----------------------------------------------------------

  pure val FRESH: Holder = { known: Set(0), storedAt: Map(0 -> -1), clock: 0, served: Set(), view: START }

  action init = s' = {
    nextId: 1,
    signed: Map(0 -> GENESIS),
    hs: HOLDERS.mapBy(_h => FRESH),
    granted: Set(),
    named: Set(),
    last: "init",
  }

  // ---- Moves ---------------------------------------------------------

  pure def add(h: Holder, i: int): Holder =
    if (h.known.contains(i)) h
    else { ...h, known: h.known.union(Set(i)), storedAt: h.storedAt.put(i, h.clock), clock: h.clock + 1 }

  pure def refresh(sg: int -> E, h: Holder): Holder = { ...h, view: viewOf(sg, h) }

  // Stores `i` and builds the holder's view again.
  pure def store(sg: int -> E, h: Holder, i: int): Holder = refresh(sg, add(h, i))

  // Signs `e` with the next id and stores it at holder `at`.
  action make(at: str, e: E, what: str): bool = {
    val e2 = { ...e, id: s.nextId }
    val sg = s.signed.put(s.nextId, e2)
    all {
      s.nextId <= MAX_IDS,
      s' = { ...s,
        nextId: s.nextId + 1,
        signed: sg,
        hs: s.hs.set(at, store(sg, s.hs.get(at), s.nextId)),
        granted: if (e.cls == "seq" and e.owner) s.granted.union(Set({ s: e.subject, l: e.level })) else s.granted,
        named: if ((e.cls == "take" or e.cls == "move") and e.owner) s.named.union(Set(e.to)) else s.named,
        last: what,
      },
    }
  }

  // The owner shares through `h`, the curator by its own view, which lists
  // the unshares of the subject it holds as seen.
  action shareAt(h: str, x: str, l: int): bool = {
    val v = s.hs.get(h).view
    all {
      v.curator == h,
      not(v.frozen),
      v.path.length() <= MAX_N,
      make(h, { ...NONE, cls: "seq", n: v.path.length(), prev: tip(v), epoch: v.epoch, curator: h, owner: true, subject: x, level: l, after: unsharesOf(s.signed, s.hs.get(h), x) }, "share"),
    }
  }

  action share = {
    nondet h = HOLDERS.oneOf()
    nondet x = SUBJECTS.oneOf()
    nondet l = LEVELS.oneOf()
    shareAt(h, x, l)
  }

  // A member signs a share as the curator, with no owner's signature.
  action rogueShare = {
    nondet h = HOLDERS.oneOf()
    nondet x = SUBJECTS.oneOf()
    nondet l = LEVELS.oneOf()
    val v = s.hs.get(h).view
    all {
      v.path.length() <= MAX_N,
      make(h, { ...NONE, cls: "seq", n: v.path.length(), prev: tip(v), epoch: v.epoch, curator: v.curator, owner: false, subject: x, level: l }, "rogue share"),
    }
  }

  // The curator signs a second entry where it signed one already.
  action equivocateAt(h: str, x: str, l: int): bool = {
    val v = s.hs.get(h).view
    all {
      v.curator == h,
      v.path.length() >= 2,
      v.path.length() - 1 > v.start,
      make(h, { ...NONE, cls: "seq", n: v.path.length() - 1, prev: v.path[v.path.length() - 2], epoch: v.epoch, curator: h, owner: true, subject: x, level: l }, "equivocate"),
    }
  }

  action equivocate = {
    nondet h = HOLDERS.oneOf()
    nondet x = SUBJECTS.oneOf()
    nondet l = LEVELS.oneOf()
    equivocateAt(h, x, l)
  }

  action unshareAt(h: str, x: str): bool =
    make(h, { ...NONE, cls: "unshare", owner: true, subject: x }, "unshare")

  action unshare = {
    nondet h = HOLDERS.oneOf()
    nondet x = SUBJECTS.oneOf()
    unshareAt(h, x)
  }

  // The owner, at `h`, takes the space to `dest` from the head `h` holds.
  action takeAt(h: str, dest: str, owner: bool): bool = {
    val v = s.hs.get(h).view
    all {
      v.epoch < MAX_EPOCH,
      make(h, { ...NONE, cls: "take", owner: owner, epoch: v.epoch + 1, to: dest, baseN: v.path.length() - 1, baseId: tip(v) }, if (owner) "take" else "rogue take"),
    }
  }

  action take = {
    nondet h = HOLDERS.oneOf()
    nondet dest = HOLDERS.oneOf()
    nondet owner = Set(true, false).oneOf()
    takeAt(h, dest, owner)
  }

  // The owner and the curator at `h` move the space to `dest`.
  action moveAt(h: str, dest: str): bool = {
    val v = s.hs.get(h).view
    all {
      v.curator == h,
      v.epoch < MAX_EPOCH,
      make(h, { ...NONE, cls: "move", owner: true, curator: h, epoch: v.epoch + 1, to: dest, baseN: v.path.length() - 1, baseId: tip(v) }, "move"),
    }
  }

  action move = {
    nondet h = HOLDERS.oneOf()
    nondet dest = HOLDERS.oneOf()
    moveAt(h, dest)
  }

  // Any signed entry reaches any holder, or never does.
  action deliverTo(h: str, i: int): bool = all {
    s.signed.keys().contains(i),
    not(s.hs.get(h).known.contains(i)),
    s' = { ...s, hs: s.hs.set(h, store(s.signed, s.hs.get(h), i)), last: "deliver" },
  }

  action deliver = {
    nondet h = HOLDERS.oneOf()
    nondet i = s.signed.keys().oneOf()
    deliverTo(h, i)
  }

  // A holder serves a subject what it has stored so far.
  action serveAt(h: str, x: str): bool = {
    val st = s.hs.get(h)
    all {
      levelOf(s.signed, st, x) > 0,
      not(st.served.contains({ s: x, at: st.clock })),
      s' = { ...s, hs: s.hs.set(h, { ...st, served: st.served.union(Set({ s: x, at: st.clock })) }), last: "serve" },
    }
  }

  action serve = {
    nondet h = HOLDERS.oneOf()
    nondet x = SUBJECTS.oneOf()
    serveAt(h, x)
  }

  action step = any { share, rogueShare, equivocate, unshare, take, move, deliver, serve }

  // ---- Invariants ----------------------------------------------------

  // 1. No widening without the owner: every level a holder gives is one
  // the owner gave that subject.
  val noWideningWithoutOwner = HOLDERS.forall(h => SUBJECTS.forall(x =>
    val l = levelOf(s.signed, s.hs.get(h), x)
    l == 0 or s.granted.contains({ s: x, l: l })))

  // 2. Every curator a holder follows is the founder or one the owner
  // named.
  val curatorFromOwner = HOLDERS.forall(h =>
    val c = s.hs.get(h).view.curator
    c == FOUNDER or s.named.contains(c))

  // 3. One curator, however the entries came: two holders with the same
  // entries have the same chain, epoch and curator, so after a partition
  // heals there is one curator everywhere.
  val agreement = HOLDERS.forall(h1 => HOLDERS.forall(h2 =>
    s.hs.get(h1).known != s.hs.get(h2).known or s.hs.get(h1).view == s.hs.get(h2).view))

  // 4. A holder that holds an unshare serves the subject nothing after
  // storing it, unless it held by then a share of the owner's that lists
  // the unshare as seen (the owner shared again).
  val unshareStopsReads = HOLDERS.forall(h =>
    val st = s.hs.get(h)
    st.served.forall(r => st.known.forall(u =>
      val e = s.signed.get(u)
      not(e.cls == "unshare" and e.owner and e.subject == r.s and st.storedAt.get(u) < r.at) or
      st.known.exists(y =>
        val f = s.signed.get(y)
        f.cls == "seq" and f.owner and f.subject == r.s and f.after.contains(u) and st.storedAt.get(y) < r.at))))

  // 5. A curator's fork applies neither side: no entry on a holder's chain
  // has another, signed by the same curator in the same epoch, at its
  // place after the same entry.
  val forkFreezes = HOLDERS.forall(h =>
    val st = s.hs.get(h)
    val v = st.view
    v.path.indices().forall(i => i == 0 or st.known.forall(j =>
      val y = s.signed.get(j)
      val x = s.signed.get(v.path[i])
      not(y.cls == "seq" and y.id != x.id and y.n == i and y.prev == v.path[i - 1] and y.epoch == x.epoch and y.curator == x.curator and y.owner))))

  val safety = and { noWideningWithoutOwner, curatorFromOwner, agreement, unshareStopsReads, forkFreezes }

  // ---- Scenarios (quint test) ----------------------------------------

  pure def viewAt(st: State, h: str): View = st.hs.get(h).view
  pure def levelAt(st: State, h: str, x: str): int = levelOf(st.signed, st.hs.get(h), x)

  action deliverAll(h: str): bool = all {
    s' = { ...s, hs: s.hs.set(h, refresh(s.signed, range(1, s.nextId).foldl(s.hs.get(h), (st, i) => add(st, i)))), last: "deliver all" },
  }

  // Two takes in a partition: the owner at B takes the space to B, at C
  // to C. After the heal, one curator everywhere, the take with the lower
  // id's.
  run twoTakesTest =
    init
      .then(shareAt("A", "S1", 1))   // 1
      .then(deliverTo("B", 1))
      .then(deliverTo("C", 1))
      .then(takeAt("B", "B", true))   // 2
      .then(takeAt("C", "C", true))   // 3
      .then(shareAt("B", "S2", 1))   // 4, by B in epoch 1
      .then(deliverAll("A"))
      .then(deliverAll("B"))
      .then(deliverAll("C"))
      .expect(and {
        viewAt(s, "A").curator == "B",
        viewAt(s, "C").curator == "B",
        levelAt(s, "C", "S2") == 1,
        safety,
      })

  // A share made beside an unshare in a partition leaves the unshare in
  // force; sharing again lists it.
  run shareBesideUnshareTest =
    init
      .then(shareAt("A", "S1", 1))   // 1
      .then(deliverTo("B", 1))
      .then(shareAt("A", "S1", 2))   // 2, wider, on A's side
      .then(unshareAt("B", "S1"))    // 3, on B's
      .then(deliverAll("A"))
      .then(deliverAll("B"))
      .expect(levelAt(s, "A", "S1") == 0 and levelAt(s, "B", "S1") == 0 and safety)
      .then(shareAt("A", "S1", 1))   // 4, after [3]
      .expect(levelAt(s, "A", "S1") == 1 and safety)

  // A share with no owner's signature, and a take that isn't the
  // owner's, change nothing.
  run rogueTest =
    init
      .then(make("A", { ...NONE, cls: "seq", n: 1, prev: 0, epoch: 0, curator: "A", owner: false, subject: "S1", level: 2 }, "rogue share"))
      .then(takeAt("B", "B", false))
      .then(deliverAll("A"))
      .expect(levelAt(s, "A", "S1") == 0 and viewAt(s, "A").curator == "A" and safety)

  // The curator signs two entries at one place: the run freezes before
  // them, and the owner's take moves on from there.
  run forkTest =
    init
      .then(shareAt("A", "S1", 1))   // 1
      .then(shareAt("A", "S2", 1))   // 2
      .then(equivocateAt("A", "S1", 2))
      .then(deliverAll("A"))
      .expect(viewAt(s, "A").frozen and viewAt(s, "A").path.length() == 2 and safety)
      .then(takeAt("A", "B", true))
      .expect(viewAt(s, "A").curator == "B" and not(viewAt(s, "A").frozen) and safety)

  // A move: the curator and the owner hand over; the old curator's later
  // entries don't count.
  run moveTest =
    init
      .then(shareAt("A", "S1", 1))   // 1
      .then(moveAt("A", "C"))        // 2, base 1
      .then(make("A", { ...NONE, cls: "seq", n: 2, prev: 1, epoch: 0, curator: "A", owner: true, subject: "S2", level: 2 }, "late share"))
      .then(deliverAll("C"))
      .expect(viewAt(s, "C").curator == "C" and levelAt(s, "C", "S2") == 0 and safety)

  // ---- Witnesses: states the runs must reach, or the invariants say little

  val someTakeWon = HOLDERS.exists(h => s.hs.get(h).view.epoch >= 1)
  val someFrozen = HOLDERS.exists(h => s.hs.get(h).view.frozen)
  // A holder holds two takes or moves of the owner's for one epoch.
  pure def competing(st: State, h: str): bool =
    val ts = st.hs.get(h).known.filter(i => (st.signed.get(i).cls == "take" or st.signed.get(i).cls == "move") and st.signed.get(i).owner)
    ts.exists(i => ts.exists(j => i != j and st.signed.get(i).epoch == st.signed.get(j).epoch))

  val someCompeting = HOLDERS.exists(h => competing(s, h))
  // Two holders that met after competing takes: what agreement is about.
  val someHealed = HOLDERS.exists(h1 => HOLDERS.exists(h2 =>
    h1 != h2 and competing(s, h1) and s.hs.get(h1).known == s.hs.get(h2).known))
  val someReshared = HOLDERS.exists(h => SUBJECTS.exists(x =>
    unsharesOf(s.signed, s.hs.get(h), x) != Set() and levelOf(s.signed, s.hs.get(h), x) > 0))
  val someUnsharedAfterServing = HOLDERS.exists(h =>
    val st = s.hs.get(h)
    st.served.exists(r => levelOf(s.signed, st, r.s) == 0 and unsharesOf(s.signed, st, r.s) != Set()))
}

// The design: what `npm run formal` checks.
module space {
  import spaceCore(BUG = "none").*
}

// Seeded bugs: each breaks one rule, and `quint run` must find the
// invariant it violates (npm run formal checks that it does).
module bugNoOwnerCheck {
  import spaceCore(BUG = "noOwnerCheck").*
}

module bugFirstTakeWins {
  import spaceCore(BUG = "firstTakeWins").*
}

module bugServeAfterUnshare {
  import spaceCore(BUG = "serveAfterUnshare").*
}

module bugNoForkFreeze {
  import spaceCore(BUG = "noForkFreeze").*
}
