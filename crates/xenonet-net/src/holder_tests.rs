// Fetching from any holder (`holder`), on the simulated network: a gap
// filled with the author away, a tampered entry and a holder that says
// nothing each skipped for the next, a Want across networks refused, a
// new member served only the live set, and a fork whose two sides two
// holders serve.

use super::*;

/// Four machines in N1, each started, and run until `script` (on a's host)
/// ends.
fn four(seed: u64, set: impl Fn(&Arc<Party>, &Arc<Party>, &Arc<Party>, &Arc<Party>)) -> (turmoil::Sim<'static>, [Arc<Party>; 4]) {
    let s = sim(seed);
    let all: &[u8] = &[A, B, C, D];
    let ps = [A, B, C, D].map(|n| Party::new(n, &[(N1, all)]));
    set(&ps[0], &ps[1], &ps[2], &ps[3]);
    (s, ps)
}

#[test]
fn with_the_author_away_another_holder_fills_the_gap() {
    let (mut s, [a, b, c, _d]) = four(21, |_, _, _, _| {});
    serve(&mut s, &b);
    serve(&mut s, &c);
    let (a2, b2, c2) = (a.clone(), b.clone(), c.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &b2).await?;
        settled(&a2, &c2).await?;
        settled(&b2, &c2).await?;
        let es: Vec<FeedEntry> = (0..6).map(|i| a2.claim(N1, i)).collect();
        for e in &es {
            a2.send(e, C);
        }
        for e in &es[..2] {
            a2.send(e, B);
        }
        until("c holds six, b two", || c2.kept_from(A, N1).len() == 6 && b2.kept_from(A, N1).len() == 2).await?;
        // a goes away.
        turmoil::partition("a", "b");
        turmoil::partition("a", "c");
        // QUIC's idle timeout ends both ends; the simulated network leaves
        // that to the test.
        a2.drop_links();
        for m in [&b2, &c2] {
            m.on_own(|l| {
                if let Some(link) = l.mesh().link(peer(A), Alpn::Link) {
                    link.close(Close::Done);
                }
            });
        }
        until("a is gone", || !b2.links().is_connected(peer(A)) && !c2.links().is_connected(peer(A))).await?;
        // b and c meet again: c's hello says it holds more of a's feed.
        b2.on_own(|l| {
            if let Some(link) = l.mesh().link(peer(C), Alpn::Link) {
                link.close(Close::Done);
            }
        });
        until("b holds all six, from c", || b2.kept_from(A, N1).len() == 6).await?;
        Ok(())
    });
    s.run().unwrap();
    let got = b.kept_from(A, N1);
    assert!(in_order(&got) && got.len() == 6, "{got:?}");
    assert!(c.served.load(Ordering::SeqCst) >= 4, "c served the four b lacked");
    assert!(b.asked.locked().iter().all(|(to, _)| *to == C));
}

/// Cuts each pair apart from the start, before any host runs.
fn cut(s: &turmoil::Sim<'static>, pairs: &[(&str, &str)]) {
    for (x, y) in pairs {
        s.partition(*x, *y);
    }
}

/// a writes four entries that c holds; b meets c, which serves badly; d,
/// connected to b already, gets the entries after b's first ask, and is
/// the next holder.
fn the_next_holder_after(holds: Holds, seed: u64) -> (Arc<Party>, Arc<Party>, Arc<Party>) {
    let (mut s, [a, b, c, d]) = four(seed, |_, _, c, _| *c.holds.locked() = holds);
    let cuts: &[(&str, &str)] = &[("a", "b")];
    serve(&mut s, &b);
    serve(&mut s, &c);
    serve(&mut s, &d);
    let (a2, b2, c2, d2) = (a.clone(), b.clone(), c.clone(), d.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &c2).await?;
        settled(&a2, &d2).await?;
        settled(&b2, &d2).await?;
        let es: Vec<FeedEntry> = (0..4).map(|i| a2.claim(N1, i)).collect();
        for e in &es {
            a2.send(e, C);
        }
        until("c holds four", || c2.kept_from(A, N1).len() == 4).await?;
        // b meets c now: its hello says it holds more of a's feed.
        b2.on_own(|l| {
            if let Some(link) = l.mesh().link(peer(C), Alpn::Link) {
                link.close(Close::Done);
            }
        });
        until("b asked c", || b2.asked.locked().iter().any(|(to, _)| *to == C)).await?;
        for e in &es {
            a2.send(e, D);
        }
        until("b holds four", || b2.kept_from(A, N1).len() == 4).await?;
        Ok(())
    });
    cut(&s, cuts);
    s.run().unwrap();
    (b, c, d)
}

#[test]
fn a_tampered_entry_from_a_holder_is_refused_and_the_next_holder_asked() {
    let (b, _c, d) = the_next_holder_after(Holds::Tampers, 22);
    assert!(b.refused.load(Ordering::SeqCst) >= 1, "the changed entry didn't check");
    assert!(b.applied.locked().iter().all(|e| e.body["path"] != "src/elsewhere.rs"), "and wasn't applied");
    assert!(d.served.load(Ordering::SeqCst) >= 4);
    let got = b.kept_from(A, N1);
    assert!(in_order(&got) && got.len() == 4, "{got:?}");
}

#[test]
fn a_holder_that_answers_nothing_is_skipped_after_ten_seconds() {
    let (b, c, d) = the_next_holder_after(Holds::Withholds, 23);
    assert_eq!(c.served.load(Ordering::SeqCst), 0);
    assert!(d.served.load(Ordering::SeqCst) >= 4);
    let asked = b.asked.locked().clone();
    let first_d = asked.iter().position(|(to, _)| *to == D).unwrap();
    assert!(asked[..first_d].iter().any(|(to, _)| *to == C), "c first, then d: {asked:?}");
}

#[test]
fn a_want_for_a_network_the_requester_isnt_in_is_refused() {
    let mut s = sim(24);
    // c is in N1 with a, and in N2 with d; d isn't in N1.
    let a = Party::new(A, &[(N1, &[A, C])]);
    let c = Party::new(C, &[(N1, &[A, C]), (N2, &[C, D])]);
    let d = Party::new(D, &[(N2, &[C, D])]);
    serve(&mut s, &c);
    serve(&mut s, &d);
    let (a2, c2, d2) = (a.clone(), c.clone(), d.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &c2).await?;
        settled(&c2, &d2).await?;
        let e = a2.claim(N1, 1);
        a2.send(&e, C);
        until("c holds it", || c2.kept_from(A, N1).len() == 1).await?;
        // d asks for N1 on the connection that carries N2 only.
        d2.links().send(peer(C), Out::Control(Control::Want { network: N1.into(), author: author(A), inc: 0, from: 1 })).ok();
        let n2 = Control::Want { network: N2.into(), author: author(A), inc: 0, from: 1 };
        d2.links().send(peer(C), Out::Control(n2)).unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok(())
    });
    s.run().unwrap();
    assert_eq!(c.served.load(Ordering::SeqCst), 0, "nothing of N1 went to d");
    assert!(d.applied.locked().is_empty(), "d took nothing of N1");
    // And the holder's own check, past the connection's: a requester its
    // chain doesn't list in N1 gets a refusal, never an entry.
    let view = c.views.locked()[N1].clone();
    let held = |_: Pos| vec![crate::holder::Held { entry: c.applied.locked()[0].clone(), open: true }];
    let out = crate::holder::serve(&view, |_| 0, peer(D), (N1, &author(A), Pos::new(0, 1)), held, &mut crate::holder::Limits::default(), T);
    assert!(matches!(&out[..], [Out::Control(Control::DontHave { why: Lack::Refused, .. })]), "{out:?}");
    // A member's Want for another network's feed, by the chain of that
    // network: refused too.
    let n2 = c.views.locked()[N2].clone();
    let out = crate::holder::serve(&n2, |_| 0, peer(D), (N1, &author(A), Pos::new(0, 1)), held, &mut crate::holder::Limits::default(), T);
    assert!(matches!(&out[..], [Out::Control(Control::DontHave { why: Lack::Refused, .. })]), "{out:?}");
}

#[test]
fn a_new_member_is_served_the_live_set_of_what_came_before_it() {
    let (mut s, [a, b, c, _d]) = four(25, |_, _, _, _| {});
    let cuts: &[(&str, &str)] = &[("a", "b"), ("b", "c")];
    serve(&mut s, &b);
    serve(&mut s, &c);
    let (a2, b2, c2) = (a.clone(), b.clone(), c.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &c2).await?;
        let one = a2.claim(N1, 1);
        let two = a2.claim(N1, 2);
        for e in [&one, &two] {
            a2.send(e, C);
        }
        // a released the first claim and compacted.
        let compact = a2.writers.locked().get_mut(N1).unwrap().compact(&a2.signer, a2.now_ms(), &[two.seq]).unwrap();
        a2.kept.locked().get_mut(N1).unwrap().push(compact.clone());
        a2.send(&compact, C);
        until("c applied the compaction", || c2.kept_from(A, N1).len() == 3).await?;
        // b was admitted after all of it, and meets c.
        tokio::time::sleep(Duration::from_secs(1)).await;
        c2.admitted.locked().insert((N1.into(), "b".into()), a2.now_ms());
        turmoil::repair("b", "c");
        settled(&b2, &c2).await?;
        until("b holds what is open", || b2.head(&author(A), N1) == compact.pos()).await?;
        Ok(())
    });
    cut(&s, cuts);
    s.run().unwrap();
    let paths: Vec<Value> = b.applied.locked().iter().filter(|e| e.kind == Kind::Claim).map(|e| e.body["path"].clone()).collect();
    assert_eq!(paths, [json!("src/2.rs")], "the open claim, never the released one");
}

#[test]
fn a_fork_split_across_two_holders_is_detected() {
    let (mut s, [a, b, c, d]) = four(26, |_, _, _, _| {});
    let cuts: &[(&str, &str)] = &[("a", "b"), ("b", "d"), ("c", "d")];
    serve(&mut s, &b);
    serve(&mut s, &c);
    serve(&mut s, &d);
    let (a2, b2, c2, d2) = (a.clone(), b.clone(), c.clone(), d.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &c2).await?;
        settled(&a2, &d2).await?;
        settled(&b2, &c2).await?;
        let shared: Vec<FeedEntry> = (0..2).map(|i| a2.claim(N1, i)).collect();
        // A clone of a's writer, as a badly restored machine would be:
        // one side goes to c, the other to d.
        let mut twin = a2.writers.locked().get(N1).unwrap().clone();
        let side: Vec<FeedEntry> = (2..4).map(|i| a2.claim(N1, i)).collect();
        let other: Vec<FeedEntry> =
            (2..5).map(|i| twin.write(&a2.signer, Kind::Claim, a2.now_ms(), json!({ "project": "prj_atlas", "path": format!("twin/{i}.rs") })).unwrap()).collect();
        for e in shared.iter().chain(&side) {
            a2.send(e, C);
        }
        for e in shared.iter().chain(&other) {
            a2.send(e, D);
        }
        until("c holds four, d five", || c2.kept_from(A, N1).len() == 4 && d2.kept_from(A, N1).len() == 5).await?;
        // b fills from c, then meets d, whose hello says it holds more.
        b2.on_own(|l| {
            if let Some(link) = l.mesh().link(peer(C), Alpn::Link) {
                link.close(Close::Done);
            }
        });
        until("b holds c's four", || b2.kept_from(A, N1).len() == 4).await?;
        turmoil::repair("b", "d");
        until("frozen", || b2.feed.locked().frozen(&author(A), N1).unwrap().is_some()).await?;
        Ok(())
    });
    cut(&s, cuts);
    s.run().unwrap();
    assert_eq!(b.kept_from(A, N1).len(), 4, "nothing of the other side is taken");
}
