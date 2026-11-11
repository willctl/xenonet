// Outbound A2A against a fake A2A 1.0 agent: a TLS server on loopback
// (std threads and rustls's own stream), with a test CA and its key made
// for these tests alone (tests/fixtures). It serves a card, answers
// SendStreamingMessage with each state a task can reach, drops one
// stream half way, and echoes the bearer in its errors, so the client's
// rules can be seen holding: the card pinned, https only, no redirect to
// another host, no loopback the person didn't allow, the interface on the
// card's host, states mapped, the bearer never in what comes back.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod fake;

use fake::{fake, roots, TOKEN};
use tokio::sync::mpsc;
use xenonet_a2a::card::{self, Pinned};
use xenonet_a2a::{Agent, Bearer, Ended, Private, Update};

async fn delegate(agent: &Agent, text: &str) -> (Result<xenonet_a2a::Delegated, xenonet_a2a::A2aError>, Vec<Update>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let r = agent.delegate(text, None, None, &tx).await;
    drop(tx);
    let mut updates = Vec::new();
    while let Some(u) = rx.recv().await {
        updates.push(u);
    }
    (r, updates)
}

#[tokio::test]
async fn a_task_streams_to_its_result() {
    let f = fake();
    let c = card::fetch(&f.url("/"), &f.allowed(), Some(&roots())).await.unwrap();
    assert_eq!(c.card.name, "Scout");
    assert!(c.interface.same_host);
    assert_eq!(c.hash.len(), 64);
    let agent = f.agent().await;
    let (r, updates) = delegate(&agent, "hello").await;
    let r = r.unwrap();
    assert!(r.stopped);
    assert_eq!((r.task.as_deref(), r.context.as_deref()), (Some("task-1"), Some("ctx-1")));
    assert!(matches!(&updates[0], Update::Working { note: None, .. }));
    assert!(matches!(&updates[1], Update::Working { note: Some(n), .. } if n.read() == "reading"));
    assert!(matches!(&updates[2], Update::Artifact { text, .. } if text.read() == "the answer"));
    assert!(matches!(&updates[3], Update::Done { text, .. } if text.read() == "the answer"), "a completed status with no words: what the artifacts said");
    let seen = f.seen.lock().unwrap();
    assert_eq!(seen.auth, vec![format!("Bearer {TOKEN}")], "the remote's bearer, and nothing of XenoNet's");
    assert_eq!(seen.methods, vec!["SendStreamingMessage"]);
    assert!(!seen.bodies[0].contains("ushNotification"), "no push config, ever");
}

#[tokio::test]
async fn task_states_map() {
    let f = fake();
    let agent = f.agent().await;
    let (_, u) = delegate(&agent, "more").await;
    assert!(matches!(u.last(), Some(Update::AsksAgent { question, .. }) if question.read() == "Which branch?"));
    let (_, u) = delegate(&agent, "sign in").await;
    assert!(matches!(u.last(), Some(Update::AsksPerson { .. })));
    let (r, u) = delegate(&agent, "fail").await;
    assert!(r.unwrap().stopped);
    let Some(Update::Ended { how, why, .. }) = u.last() else { panic!("{u:?}") };
    assert_eq!(*how, Ended::Failed);
    assert!(!why.read().contains(TOKEN) && why.read().contains("[token]"), "{}", why.read());
    let canceled = agent.cancel("task-1").await.unwrap();
    assert!(matches!(canceled, Update::Ended { how: Ended::Canceled, .. }));
}

#[tokio::test]
async fn a_dropped_stream_is_picked_up_again_not_polled() {
    let f = fake();
    let agent = f.agent().await;
    let (r, u) = delegate(&agent, "drop").await;
    assert!(r.unwrap().stopped);
    assert!(matches!(u.last(), Some(Update::Done { text, .. }) if text.read() == "picked up"));
    assert_eq!(f.seen.lock().unwrap().methods, vec!["SendStreamingMessage", "SubscribeToTask"], "never GetTask");
}

#[tokio::test]
async fn the_bearer_never_comes_back() {
    let f = fake();
    let agent = f.agent().await;
    let (r, u) = delegate(&agent, "echo").await;
    let e = r.unwrap_err();
    assert_eq!(e.code, "remote");
    assert!(!e.message.contains(TOKEN), "{}", e.message);
    assert!(!format!("{e:?}").contains(TOKEN));
    assert!(u.is_empty());
    let wrong =
        Agent::new(Pinned::of(&card::fetch(&f.url("/"), &f.allowed(), Some(&roots())).await.unwrap(), f.allowed()), Bearer::new("tok-wrong-000000".into())).trusting(roots());
    let (r, _) = delegate(&wrong, "hello").await;
    let e = r.unwrap_err();
    assert!(e.message.contains("refused its token"), "{}", e.message);
    assert!(!e.message.contains("tok-wrong"));
}

#[tokio::test]
async fn a_card_over_http_is_refused() {
    let f = fake();
    let e = card::fetch(&format!("http://localhost:{}/", f.port), &f.allowed(), Some(&roots())).await.unwrap_err();
    assert_eq!(e.code, "http");
}

#[tokio::test]
async fn a_redirect_to_another_host_is_refused() {
    let f = fake();
    let e = card::fetch(&f.url("/elsewhere"), &f.allowed(), Some(&roots())).await.unwrap_err();
    assert_eq!(e.code, "redirect", "{}", e.message);
    // One within the card's host is followed.
    let c = card::fetch(&f.url("/here"), &f.allowed(), Some(&roots())).await.unwrap();
    assert_eq!(c.url.path(), "/.well-known/agent-card.json");
}

#[tokio::test]
async fn loopback_is_refused_unless_the_person_allowed_it() {
    let f = fake();
    let e = card::fetch(&f.url("/"), &Private::none(), Some(&roots())).await.unwrap_err();
    assert_eq!(e.code, "private", "localhost resolves to loopback: {}", e.message);
    let e = card::fetch(&format!("https://127.0.0.1:{}/", f.port), &Private::none(), Some(&roots())).await.unwrap_err();
    assert_eq!(e.code, "private", "https://127.0.0.1 itself");
    let other = Private { host: Some("localhost:1".into()) };
    assert_eq!(card::fetch(&f.url("/"), &other, Some(&roots())).await.unwrap_err().code, "private", "allowed for another port only");
}

#[tokio::test]
async fn an_interface_on_another_host_needs_the_persons_approval() {
    let f = fake();
    f.seen.lock().unwrap().interface = Some(format!("127.0.0.1:{}", f.port));
    let c = card::fetch(&f.url("/"), &f.allowed(), Some(&roots())).await.unwrap();
    assert!(!c.interface.same_host);
    assert_eq!(c.interface.host, format!("127.0.0.1:{}", f.port));
    let unapproved = Pinned { approved_host: None, ..Pinned::of(&c, f.allowed()) };
    let (r, _) = delegate(&Agent::new(unapproved, Bearer::new(TOKEN.into())).trusting(roots()), "hello").await;
    assert_eq!(r.unwrap_err().code, "host");
    assert!(f.seen.lock().unwrap().methods.is_empty(), "nothing was sent");
}

#[tokio::test]
async fn a_changed_card_is_refused_until_approved_again() {
    let f = fake();
    let agent = f.agent().await;
    f.seen.lock().unwrap().version = "2.0.0".into();
    let (r, _) = delegate(&agent, "hello").await;
    assert_eq!(r.unwrap_err().code, "changed");
    assert!(f.seen.lock().unwrap().methods.is_empty(), "nothing was sent");
}
