// Mailboxes (`mailbox`) on the simulated network: a mailbox that dies
// mid-transfer, an evicted entry that shows as a gap, a receipt from a
// mailbox the network doesn't list, and a member of another network that
// can't fill this one's queues.

use super::*;
use crate::mailbox::Standing;

/// Cuts each pair apart from the start.
fn cut(s: &turmoil::Sim<'static>, pairs: &[(&str, &str)]) {
    for (x, y) in pairs {
        s.partition(*x, *y);
    }
}

/// a, b and c in N1, with c its mailbox.
fn three(seed: u64) -> (turmoil::Sim<'static>, [Arc<Party>; 3]) {
    let all: &[u8] = &[A, B, C];
    let ps = [A, B, C].map(|n| Party::new(n, &[(N1, all)]));
    for p in &ps {
        p.mailboxes(N1, &[C]);
    }
    (sim(seed), ps)
}

fn mail_for(m: &Party, network: &str, to: u8) -> usize {
    crate::persist::Persist::mail_for(&m.mail, network, &author(to), usize::MAX).unwrap().len()
}

fn messages_from(m: &Party, from: u8) -> Vec<String> {
    let a = author(from);
    m.applied.locked().iter().filter(|e| e.author == a && e.kind == Kind::Message).map(|e| e.body["text"].as_str().unwrap_or("").to_string()).collect()
}

#[test]
fn a_mailbox_that_dies_mid_transfer_hands_over_the_rest_when_back() {
    let (mut s, [a, b, c]) = three(31);
    serve(&mut s, &b);
    serve(&mut s, &c);
    let (a2, b2, c2) = (a.clone(), b.clone(), c.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &c2).await?;
        let mut sent = Vec::new();
        for i in 0..6 {
            sent.push(a2.claim(N1, i));
        }
        for i in 0..2 {
            sent.push(a2.write(N1, Kind::Message, json!({ "to": "b", "text": format!("m{i}") })));
        }
        for e in &sent {
            assert_eq!(a2.hand_off(e, B), 1, "b is away: to the mailbox");
        }
        until("c keeps all eight, and a has its receipts", || mail_for(&c2, N1, B) == 8 && a2.standing(N1, B) == Standing { held: 6, unreceipted: 0 }).await?;
        // a goes away for good.
        turmoil::partition("a", "c");
        a2.drop_links();
        c2.on_own(|l| {
            if let Some(link) = l.mesh().link(peer(A), Alpn::Link) {
                link.close(Close::Done);
            }
        });
        // b comes back; c hands over three at a time, and dies after the
        // first three.
        c2.post.locked().handing.limit = 3;
        turmoil::repair("b", "c");
        until("b took three", || b2.kept_from(A, N1).len() >= 3).await?;
        turmoil::partition("b", "c");
        for m in [&b2, &c2] {
            m.drop_links();
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        turmoil::repair("b", "c");
        until("b holds all six and both messages", || b2.kept_from(A, N1).len() == 6 && messages_from(&b2, A).len() == 2).await?;
        until("c's queue for b is empty", || mail_for(&c2, N1, B) == 0).await?;
        Ok(())
    });
    cut(&s, &[("a", "b"), ("b", "c")]);
    s.run().unwrap();
    let got = b.kept_from(A, N1);
    assert!(in_order(&got) && got.len() == 6, "{got:?}");
    assert_eq!(messages_from(&b, A), ["m0", "m1"], "each message once");
}

#[test]
fn an_evicted_entry_shows_as_a_gap_that_is_filled_never_skipped() {
    let (mut s, [a, b, c]) = three(32);
    *c.caps.locked() = crate::mailbox::Caps { entries: 3, ..crate::mailbox::Caps::default() };
    serve(&mut s, &b);
    serve(&mut s, &c);
    let (a2, b2, c2) = (a.clone(), b.clone(), c.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &c2).await?;
        let es: Vec<FeedEntry> = (0..6).map(|i| a2.claim(N1, i)).collect();
        for e in &es {
            a2.hand_off(e, B);
        }
        until("c kept the newest three", || mail_for(&c2, N1, B) == 3 && a2.standing(N1, B).held == 6).await?;
        // b comes back to c alone: 4, 5 and 6 come, and wait on a gap.
        turmoil::repair("b", "c");
        until("b asked for the gap", || b2.wants.load(Ordering::SeqCst) >= 1).await?;
        assert_eq!(b2.kept_from(A, N1).len(), 0, "nothing past the gap applied");
        // a, back, fills it.
        turmoil::repair("a", "b");
        until("b holds all six", || b2.kept_from(A, N1).len() == 6).await?;
        until("a sees them delivered", || a2.standing(N1, B) == Standing::default()).await?;
        Ok(())
    });
    cut(&s, &[("a", "b"), ("b", "c")]);
    s.run().unwrap();
    let got = b.kept_from(A, N1);
    assert_eq!(got.iter().map(|p| p.seq).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
}

#[test]
fn a_receipt_from_a_mailbox_the_network_doesnt_list_is_dropped() {
    let all: &[u8] = &[A, B, C, D];
    let [a, b, c, d] = [A, B, C, D].map(|n| Party::new(n, &[(N1, all)]));
    for p in [&a, &b, &c, &d] {
        p.mailboxes(N1, &[C]);
    }
    let mut s = sim(33);
    serve(&mut s, &c);
    serve(&mut s, &d);
    let (a2, c2, d2) = (a.clone(), c.clone(), d.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &c2).await?;
        settled(&a2, &d2).await?;
        let e1 = a2.claim(N1, 1);
        assert_eq!(a2.hand_off(&e1, B), 1, "to c alone: d isn't a mailbox");
        until("c's receipt", || a2.standing(N1, B).held == 1).await?;
        // d says it holds another entry of a's for b, signed as well as it
        // can be.
        let e2 = a2.claim(N1, 2);
        a2.post.locked().handed.handed(N1, &author(B), &e2.id(), e2.pos());
        let mut h = crate::mailbox::Held {
            network: N1.into(),
            mailbox: "d".into(),
            author: a2.me(),
            items: vec![crate::mailbox::HeldItem { id: e2.id(), to: author(B), at: T }],
            at: T,
            sig: Vec::new(),
        };
        h.sign(&d2.signer).unwrap();
        d2.links().send(peer(A), Out::Control(Control::Held { network: N1.into(), body: h.to_json() })).unwrap();
        until("refused", || a2.refused.load(Ordering::SeqCst) >= 1).await?;
        Ok(())
    });
    // b never runs: it is away throughout.
    s.run().unwrap();
    assert_eq!(a.standing(N1, B), Standing { held: 1, unreceipted: 1 }, "d's receipt counted for nothing");
    let _ = b;
}

#[test]
fn a_member_of_another_network_cant_fill_this_networks_queues() {
    // N1: a, b and c; N2: b, c and d. c is the mailbox of both.
    let a = Party::new(A, &[(N1, &[A, B, C])]);
    let b = Party::new(B, &[(N1, &[A, B, C]), (N2, &[B, C, D])]);
    let c = Party::new(C, &[(N1, &[A, B, C]), (N2, &[B, C, D])]);
    let d = Party::new(D, &[(N2, &[B, C, D])]);
    for (p, nets) in [(&a, &[N1][..]), (&b, &[N1, N2]), (&c, &[N1, N2]), (&d, &[N2])] {
        for n in nets {
            p.mailboxes(n, &[C]);
        }
    }
    *c.caps.locked() = crate::mailbox::Caps { entries: 3, ..crate::mailbox::Caps::default() };
    let mut s = sim(34);
    serve(&mut s, &c);
    serve(&mut s, &d);
    let (a2, c2, d2) = (a.clone(), c.clone(), d.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &c2).await?;
        settled(&d2, &c2).await?;
        for i in 0..3 {
            let e = a2.claim(N1, i);
            assert_eq!(a2.hand_off(&e, B), 1);
        }
        until("c keeps a's three for b in N1", || mail_for(&c2, N1, B) == 3).await?;
        // d floods b's queue in N2, the network it shares with c.
        for i in 0..10 {
            let e = d2.claim(N2, i);
            assert_eq!(d2.hand_off(&e, B), 1);
        }
        until("N2's queue is full", || mail_for(&c2, N2, B) == 3).await?;
        // And can't reach N1's: its connection to c carries N2 alone.
        let e = d2.claim(N2, 99);
        let mut n1 = Party::seal(&e, B);
        n1.network = N1.into();
        assert!(d2.links().send(peer(C), Out::Envelope(n1)).is_err());
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(())
    });
    // b never runs: it is away throughout.
    s.run().unwrap();
    let kept: Vec<String> = crate::persist::Persist::mail_for(&c.mail, N1, &author(B), 10).unwrap().into_iter().map(|m| m.author).collect();
    assert_eq!(kept, [author(A), author(A), author(A)], "N1's queue is a's alone, untouched by d");
    let _ = b;
}
