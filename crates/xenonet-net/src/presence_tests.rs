// Presence on link/2 (`presence`) on the simulated network: it converges
// over dropped connections, each sending a snapshot on connect and deltas
// on change, and a peer's presence ends 30 s after its connection goes.

use super::*;

fn body(agents: &BTreeMap<String, Value>, you: bool) -> Value {
    json!({ "schema": crate::presence::SCHEMA, "counter": 0, "full": true, "agents": agents.values().cloned().collect::<Vec<_>>(), "removed": [], "you": you, "roster": "head" })
}

#[test]
fn presence_converges_over_dropped_connections_and_ends_thirty_seconds_after_one_goes() {
    for seed in [41, 42] {
        let mut s = sim(seed);
        let a = Party::new(A, &[(N1, &[A, B])]);
        let b = Party::new(B, &[(N1, &[A, B])]);
        serve(&mut s, &b);
        let (a2, b2) = (a.clone(), b.clone());
        s.client("a", async move {
            a2.start().await?;
            settled(&a2, &b2).await?;
            let mut agents = BTreeMap::new();
            for round in 0..40u64 {
                let id = format!("claude:{}", round % 7);
                agents.insert(id.clone(), json!({ "id": id, "status": format!("s{round}") }));
                if round % 5 == 4 {
                    agents.remove(&format!("claude:{}", (round + 3) % 7));
                }
                a2.say(N1, &body(&agents, round % 2 == 0));
                // The connection drops now and then: what was said while
                // it was down comes whole on the next.
                if round % 8 == 7 {
                    if let Some(l) = a2.links().mesh().link(peer(B), Alpn::Link) {
                        l.close(Close::Done);
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let want = json!(agents.values().cloned().collect::<Vec<_>>());
            until("b holds a's presence as it is", || b2.view_of(A).as_ref() == Some(&want)).await?;
            let sent = b2.live.locked().len();
            // Nothing changes: nothing goes.
            a2.say(N1, &body(&agents, true));
            a2.say(N1, &body(&agents, true));
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert!(b2.live.locked().len() <= sent + 1, "at most the one change of `you`");
            // a goes away for good: b drops its presence 30 s after.
            turmoil::partition("a", "b");
            for m in [&a2, &b2] {
                m.drop_links();
            }
            until("b's end closed", || !b2.links().is_connected(peer(A))).await?;
            let gone = turmoil::elapsed();
            tokio::time::sleep(Duration::from_secs(25)).await;
            assert!(b2.view_of(A).is_some(), "still held 25 s after");
            until("b dropped it", || b2.view_of(A).is_none()).await?;
            let took = turmoil::elapsed().saturating_sub(gone);
            assert!(took >= Duration::from_millis(29_900) && took < Duration::from_secs(31), "{took:?}");
            Ok(())
        });
        s.run().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
    }
}
