//! The client against the fake BiDi server, over an in-memory stream.

use std::time::Duration;

use eidolon_bidi::{Error, Options, Session, Wait};
use eidolon_testkit::{screenshot_bytes, FakeBidi, Page};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const HOST: &str = "127.0.0.1:9222";
const LIMIT: Duration = Duration::from_secs(5);

type Served = JoinHandle<Result<(), Error>>;

async fn connect(fake: FakeBidi) -> (Session, Page, Served) {
    connect_with(fake, Options::default()).await
}

async fn connect_with(fake: FakeBidi, options: Options) -> (Session, Page, Served) {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let page = fake.page();
    let served = tokio::spawn(fake.serve(server));
    let session = timeout(LIMIT, Session::connect_with(client, HOST, options)).await.expect("the handshake does not hang").expect("the handshake works");
    (session, page, served)
}

#[tokio::test]
async fn handshake_then_ready() {
    let (session, _, _) = connect(FakeBidi::new()).await;
    session.wait_ready(LIMIT).await.unwrap();
}

#[tokio::test]
async fn a_bad_accept_key_is_refused() {
    let (client, server) = tokio::io::duplex(4096);
    tokio::spawn(FakeBidi::new().bad_accept().serve(server));
    let err = timeout(LIMIT, Session::connect(client, HOST)).await.unwrap().err();
    assert!(matches!(err, Some(Error::Handshake(_))), "{err:?}");
}

#[tokio::test]
async fn a_command_gets_its_answer() {
    let (session, _, _) = connect(FakeBidi::new()).await;
    let info = timeout(LIMIT, session.new_session()).await.unwrap().unwrap();
    assert_eq!(info["sessionId"], "fake-session");
}

#[tokio::test]
async fn concurrent_commands_each_get_their_own_answer() {
    let (session, _, _) = connect(FakeBidi::new()).await;
    let session = std::sync::Arc::new(session);
    let mut tasks = Vec::new();
    for n in 0..40 {
        let session = session.clone();
        tasks.push(tokio::spawn(async move {
            let answer = session.send("fake.echo", json!({ "n": n })).await.unwrap();
            assert_eq!(answer["n"], n);
        }));
    }
    for task in tasks {
        timeout(LIMIT, task).await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn an_error_answer_is_typed() {
    let fake = FakeBidi::new().fail("browsingContext.navigate", "unknown error", "net::ERR_FAILED");
    let (session, _, _) = connect(fake).await;
    let err = session.navigate("ctx-1", "https://example.test/", Wait::Complete).await.unwrap_err();
    assert_eq!(err, Error::Remote { code: "unknown error".into(), message: "net::ERR_FAILED".into() });
    // The connection still works.
    session.wait_ready(LIMIT).await.unwrap();
}

#[tokio::test]
async fn an_unknown_context_is_the_browsers_error() {
    let (session, _, _) = connect(FakeBidi::new()).await;
    let err = session.capture_screenshot("ctx-99").await.unwrap_err();
    assert!(matches!(err, Error::Remote { ref code, .. } if code == "no such frame"), "{err:?}");
}

#[tokio::test]
async fn events_reach_a_subscriber() {
    let (session, _, _) = connect(FakeBidi::new()).await;
    let mut events = session.subscribe(&["browsingContext.load"]).await.unwrap();
    session.navigate("ctx-1", "https://example.test/a", Wait::Complete).await.unwrap();
    let event = timeout(LIMIT, events.recv()).await.unwrap().unwrap();
    assert_eq!(event.method, "browsingContext.load");
    assert_eq!(event.params["url"], "https://example.test/a");
}

#[tokio::test]
async fn a_subscriber_gets_only_what_it_asked_for() {
    let (session, _, _) = connect(FakeBidi::new()).await;
    let mut other = session.subscribe(&["log.entryAdded"]).await.unwrap();
    let mut module = session.subscribe(&["browsingContext"]).await.unwrap();
    session.navigate("ctx-1", "https://example.test/", Wait::Complete).await.unwrap();
    assert_eq!(timeout(LIMIT, module.recv()).await.unwrap().unwrap().method, "browsingContext.load");
    assert!(other.try_recv().is_err());
}

#[tokio::test]
async fn a_fragmented_answer_is_put_together_and_the_ping_between_is_answered() {
    let (session, page, _) = connect(FakeBidi::new().fragment_answers()).await;
    let info = timeout(LIMIT, session.new_session()).await.unwrap().unwrap();
    assert_eq!(info["sessionId"], "fake-session");
    session.wait_ready(LIMIT).await.unwrap();
    assert!(page.pongs() >= 1);
}

#[tokio::test]
async fn a_ping_is_answered_with_a_pong() {
    let (session, page, _) = connect(FakeBidi::new().ping_first()).await;
    // Two commands: the pong is written before the second one is read.
    session.wait_ready(LIMIT).await.unwrap();
    session.wait_ready(LIMIT).await.unwrap();
    assert_eq!(page.pongs(), 1);
}

#[tokio::test]
async fn a_message_over_the_limit_is_refused() {
    let fake = FakeBidi::new().with_title(&"x".repeat(10_000));
    let options = Options { max_message: 4096, ..Options::default() };
    let (session, _, _) = connect_with(fake, options).await;
    session.wait_ready(LIMIT).await.unwrap();
    let err = session.script_evaluate("ctx-1", "document.title", false).await.unwrap_err();
    assert_eq!(err, Error::MessageTooLarge { limit: 4096 });
    // The connection is closed after that.
    let again = session.wait_ready(LIMIT).await.unwrap_err();
    assert_eq!(again, err);
}

#[tokio::test]
async fn a_dropped_connection_fails_the_pending_command_and_the_next_ones() {
    let (session, _, _) = connect(FakeBidi::new().drop_on_message(2)).await;
    session.wait_ready(LIMIT).await.unwrap();
    let err = timeout(LIMIT, session.wait_ready(LIMIT)).await.expect("no hang").unwrap_err();
    assert!(matches!(err, Error::Closed(_) | Error::Io(_)), "{err:?}");
    let next = timeout(LIMIT, session.new_session()).await.expect("no hang").unwrap_err();
    assert_eq!(next, err);
}

#[tokio::test]
async fn a_dropped_connection_ends_an_event_channel() {
    let (session, _, _) = connect(FakeBidi::new().drop_on_message(2)).await;
    let mut events = session.subscribe(&["browsingContext.load"]).await.unwrap();
    let _ = session.wait_ready(LIMIT).await;
    assert!(timeout(LIMIT, events.recv()).await.expect("no hang").is_none());
}

#[tokio::test]
async fn closing_politely_ends_the_server_cleanly() {
    let (session, _, served) = connect(FakeBidi::new()).await;
    session.wait_ready(LIMIT).await.unwrap();
    timeout(LIMIT, session.close()).await.unwrap().unwrap();
    assert_eq!(timeout(LIMIT, served).await.unwrap().unwrap(), Ok(()));
}

#[tokio::test]
async fn a_script_that_throws_is_a_script_error() {
    let (session, _, _) = connect(FakeBidi::new()).await;
    let err = session.script_evaluate("ctx-1", "throw Error: boom", false).await.unwrap_err();
    assert_eq!(err, Error::Script("Error: boom".into()));
}

#[tokio::test]
async fn tabs_open_list_and_close() {
    let (session, _, _) = connect(FakeBidi::new()).await;
    let tab = session.browsing_context_create().await.unwrap();
    let tree = session.browsing_context_get_tree(None).await.unwrap();
    assert_eq!(tree.iter().map(|c| c.context.as_str()).collect::<Vec<_>>(), ["ctx-1", tab.as_str()]);
    session.browsing_context_close(&tab).await.unwrap();
    assert_eq!(session.browsing_context_get_tree(None).await.unwrap().len(), 1);
    session.session_end().await.unwrap();
}

#[tokio::test]
async fn navigate_locate_click_type_set_files_and_screenshot() {
    let (session, page, _) = connect(FakeBidi::new()).await;
    session.new_session().await.unwrap();
    let nav = session.navigate("ctx-1", "https://example.test/form", Wait::Complete).await.unwrap();
    assert_eq!(nav.url, "https://example.test/form");
    assert_eq!(page.url(), "https://example.test/form");

    let title = session.script_evaluate("ctx-1", "document.title", false).await.unwrap();
    assert_eq!(title, json!({ "type": "string", "value": "Page at https://example.test/form" }));

    let field = session.locate_nodes("ctx-1", "#name").await.unwrap();
    assert_eq!(field.len(), 1);
    session.click("ctx-1", &field[0].shared_id).await.unwrap();
    session.type_text("ctx-1", "hello").await.unwrap();
    assert_eq!(page.field_value("#name").as_deref(), Some("hello"));
    let value = session.script_evaluate("ctx-1", "document.querySelector('#name').value", false).await.unwrap();
    assert_eq!(value["value"], "hello");

    let button = session.locate_nodes("ctx-1", "#go").await.unwrap();
    session.click("ctx-1", &button[0].shared_id).await.unwrap();
    session.click("ctx-1", &button[0].shared_id).await.unwrap();
    assert_eq!(page.clicks_on("#go"), 2);

    let upload = session.locate_nodes("ctx-1", "#upload").await.unwrap();
    session.input_set_files("ctx-1", &upload[0].shared_id, &["/tmp/a.txt", "/tmp/b.txt"]).await.unwrap();
    assert_eq!(page.files("#upload"), ["/tmp/a.txt", "/tmp/b.txt"]);
    // A file can only go in a file input.
    let err = session.input_set_files("ctx-1", &button[0].shared_id, &["/tmp/a.txt"]).await.unwrap_err();
    assert!(matches!(err, Error::Remote { ref code, .. } if code == "unable to set file input"), "{err:?}");

    assert_eq!(session.capture_screenshot("ctx-1").await.unwrap(), screenshot_bytes());
    assert_eq!(session.locate_nodes("ctx-1", "#nothing").await.unwrap().len(), 0);

    let result: Value = session.script_call_function("ctx-1", "() => 1", vec![], false).await.unwrap();
    assert_eq!(result["type"], "undefined");
}
