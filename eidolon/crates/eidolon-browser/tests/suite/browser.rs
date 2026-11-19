//! Reaching the browser as a stream, and reconnecting when it restarts.

use crate::support::{cell, FakeDriver, Next};
use eidolon_bidi::Wait;
use eidolon_browser::{Browser, BrowserError, ConnectOptions};
use eidolon_testkit::{FakeBidi, NodeKind};
use serde_json::json;
use std::time::Duration;

fn quick() -> ConnectOptions {
    ConnectOptions { attempts: 5, gap: Duration::ZERO, ready_within: Duration::from_secs(5) }
}

#[tokio::test]
async fn connect_reaches_the_port_as_a_stream_and_waits_for_the_protocol() {
    let fake = FakeBidi::new().with_node("#go", NodeKind::Button);
    let page = fake.page();
    let driver = FakeDriver::new(vec![Next::Serve(fake)]);
    let session = Browser::connect(driver.as_ref(), &cell(), 9222).await.unwrap();

    // The only way in is socat on the guest's own loopback.
    assert_eq!(driver.streams.lock().unwrap().clone(), vec![vec!["socat".to_owned(), "-".into(), "TCP:127.0.0.1:9222".into()]]);
    // Ready meant the browser answered, so the first thing it saw is the status call.
    assert_eq!(page.methods(), ["session.status"]);

    session.new_session().await.unwrap();
    let context = session.browsing_context_get_tree(None).await.unwrap()[0].context.clone();
    session.navigate(&context, "http://127.0.0.1:8000/", Wait::Complete).await.unwrap();
    let png = session.capture_screenshot(&context).await.unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    assert!(driver.execs().is_empty(), "nothing but the stream ran in the cell");
}

#[tokio::test]
async fn connect_tries_the_stream_again_while_the_browser_starts() {
    // Nothing listens twice (socat fails, then the stream ends), then it does.
    let driver = FakeDriver::new(vec![Next::Tool, Next::Closed, Next::Serve(FakeBidi::new())]);
    let session = Browser::connect_with(driver.as_ref(), &cell(), 9222, quick()).await.unwrap();
    assert_eq!(driver.stream_count(), 3);
    session.status().await.unwrap();
}

#[tokio::test]
async fn connect_gives_up_after_its_bound_and_says_what_it_last_saw() {
    let driver = FakeDriver::new((0..10).map(|_| Next::Tool).collect());
    let err = Browser::connect_with(driver.as_ref(), &cell(), 9222, quick()).await.err().unwrap();
    assert!(matches!(err, BrowserError::GaveUp { attempts: 5, .. }), "{err}");
    assert_eq!(driver.stream_count(), 5);
    assert!(err.to_string().contains("connection refused"));
}

#[tokio::test]
async fn an_error_that_trying_again_cannot_fix_is_not_retried() {
    let driver = FakeDriver::new(vec![Next::Unsupported, Next::Serve(FakeBidi::new())]);
    let err = Browser::connect_with(driver.as_ref(), &cell(), 9222, quick()).await.err().unwrap();
    assert!(matches!(err, BrowserError::Driver(_)), "{err}");
    assert_eq!(driver.stream_count(), 1);
}

#[tokio::test]
async fn a_browser_that_says_it_is_not_ready_is_an_error_after_the_bound() {
    let driver = FakeDriver::new(vec![Next::Serve(FakeBidi::new().fail("session.status", "unknown error", "starting"))]);
    let err = Browser::connect_with(driver.as_ref(), &cell(), 9222, ConnectOptions { attempts: 1, ..quick() }).await.err().unwrap();
    assert!(err.to_string().contains("starting"), "{err}");
}

#[tokio::test]
async fn after_the_connection_drops_the_next_command_succeeds_on_a_new_one() {
    // Message 1 is the readiness check, 2 opens the session; message 3
    // drops the connection.
    let first = FakeBidi::new().drop_on_message(3);
    let second = FakeBidi::new();
    let second_page = second.page();
    let driver = FakeDriver::new(vec![Next::Serve(first), Next::Serve(second)]);
    let browser = Browser::supervise(driver.clone(), cell(), 9222, quick());

    let lost = browser.send("session.status", json!({})).await;
    assert!(lost.is_err(), "the command that met the drop is not repeated for the caller");
    assert_eq!(driver.stream_count(), 1);

    let answer = browser.send("session.status", json!({})).await.unwrap();
    assert_eq!(answer["ready"], true);
    assert_eq!(driver.stream_count(), 2, "a second connection was made");
    // The new browser was asked if it is ready and given a session before it got the command.
    assert_eq!(second_page.methods(), ["session.status", "session.new", "session.status"]);
}

#[tokio::test]
async fn a_subscription_survives_a_reconnect() {
    let first = FakeBidi::new().drop_on_message(4);
    let second = FakeBidi::new();
    let second_page = second.page();
    let driver = FakeDriver::new(vec![Next::Serve(first), Next::Serve(second)]);
    let browser = Browser::supervise(driver.clone(), cell(), 9222, quick());

    // Messages: 1 status, 2 new session, 3 subscribe, 4 (dropped).
    let mut events = browser.subscribe(&["browsingContext.load"]).await.unwrap();
    assert!(browser.send("session.status", json!({})).await.is_err());

    let session = browser.session().await.unwrap();
    assert_eq!(driver.stream_count(), 2);
    assert!(second_page.methods().contains(&"session.subscribe".to_owned()), "subscribed again: {:?}", second_page.methods());

    let context = session.browsing_context_get_tree(None).await.unwrap()[0].context.clone();
    session.navigate(&context, "http://127.0.0.1:8000/", Wait::None).await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
    assert_eq!(event.method, "browsingContext.load");
}

#[tokio::test]
async fn a_remote_error_does_not_drop_the_connection() {
    let driver = FakeDriver::new(vec![Next::Serve(FakeBidi::new())]);
    let browser = Browser::supervise(driver.clone(), cell(), 9222, quick());
    assert!(browser.send("nonsense.method", json!({})).await.is_err());
    browser.send("session.status", json!({})).await.unwrap();
    assert_eq!(driver.stream_count(), 1);
}
