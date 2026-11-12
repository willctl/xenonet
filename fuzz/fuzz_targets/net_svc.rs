#![no_main]

// Services through the network (docs/design/network.md, "Services
// through the network"): what a publishing host reads of another host's
// svc/2 stream before it passes it to a service, and what a caller's host
// reads of the answer. The first byte picks the input's shape:
//
// - 0: a path after /api/svc/<host>/<name>, normalized against an
//   allowlist.
// - 1: a request head (network/svc/v2), the schema's own with the input's
//   fields over it, or the input's JSON alone.
// - 2: an answer head, the same way.
// - 3: a published service's address.
// - 4: headers, as the caller and the service send them.
// - 5: a window read's head, and the path a window may read.
//
// Nothing may panic. A path that passes never holds what a server could
// read as another path, and normalizing it again changes nothing; the
// service is sent only the caller's four headers, its Host and one
// Forwarded; an answer never carries Set-Cookie or a hop-by-hop header;
// an address that passes is this machine's, over http; and a window reads
// a session's transcript or stats alone.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use xenonet_fuzz::net::{overlay, Choices};
use xenonet_net::svc::{answer_headers, normalize, request_headers, Answer, Published, Request, Target, PASSED};
use xenonet_net::svc2::{window_path, WindowRead};

fn path(text: &str) {
    let Ok(n) = normalize(text) else { return };
    assert!(n.decoded.starts_with('/'), "{n:?}");
    assert!(!n.decoded.contains("//") && !n.decoded.contains(';') && !n.decoded.contains('\\'), "{n:?}");
    assert!(!n.decoded.split('/').any(|s| s == "." || s == ".."), "{n:?}");
    assert!(!n.decoded.chars().any(char::is_control), "{n:?}");
    assert!(n.path.bytes().all(|b| (0x21..=0x7e).contains(&b)) && !n.path.contains(['?', '#', ';', '\\']), "{n:?}");
    let again = normalize(&n.path).expect("a normalized path passes again");
    assert_eq!((again.decoded, again.path), (n.decoded.clone(), n.path.clone()), "normalizing twice changed it");
    let p = Published::new("files", "http://127.0.0.1:4790", &["/sync/v1/".into()], &[], 0).expect("a service");
    if p.reaches(&n.decoded) {
        assert!(n.target("").starts_with("/sync/v1/"), "{n:?}");
    }
}

fn request(v: &Value) {
    let Ok((r, length)) = Request::parse_head(v) else { return };
    let r = Request { body: vec![0; length.min(1 << 16)], ..r };
    let (back, n) = Request::parse_head(&r.head("net_00112233445566778899aabbccddeeff")).expect("a request's own head parses");
    assert_eq!((Request { body: r.body.clone(), ..back }, n), (r.clone(), r.body.len()), "a request reads back as another");
}

fn answer(v: &Value) {
    let Ok((a, length)) = Answer::parse_head(v) else { return };
    assert!((200..=599).contains(&a.status), "{a:?}");
    for (k, _) in &a.headers {
        assert!(!matches!(k.as_str(), "set-cookie" | "set-cookie2" | "connection" | "keep-alive" | "transfer-encoding" | "upgrade" | "te" | "trailer" | "content-length"), "{k}");
    }
    assert_eq!(answer_headers(&a.headers), a.headers, "cleaning twice changed it");
    let a = Answer { body: vec![0; length.min(1 << 16)], ..a };
    let (b, n) = Answer::parse_head(&a.head()).expect("an answer's own head parses");
    assert_eq!((Answer { body: a.body.clone(), ..b }, n), (a.clone(), a.body.len()), "an answer reads back as another");
}

fn window(v: &Value, titles_only: bool) {
    if let Ok(r) = WindowRead::parse(v) {
        assert_eq!(WindowRead::parse(&r.head()).as_ref(), Ok(&r), "a window read reads back as another");
    }
    let Some(path) = v.get("path").and_then(Value::as_str) else { return };
    let Ok(p) = window_path(path, titles_only) else { return };
    assert!(p.starts_with("/api/session/") && p.split('/').nth(3).is_some_and(|s| s.contains(':')), "{p}");
    assert!(p.ends_with("/stats") || p.contains("/transcript"), "{p}");
    assert!(!(titles_only && p.contains("/transcript")), "{p}");
    assert!(!p.contains("..") && !p.contains('%') && !p.contains('#'), "{p}");
}

fn target(text: &str) {
    let Ok(t) = Target::parse(text) else { return };
    assert!(t.ip.is_loopback(), "{t:?}");
    assert_eq!(Target::parse(&t.url()).as_ref(), Ok(&t), "an address reads back as another");
}

fn headers(c: &mut Choices<'_>) {
    let names = [
        "authorization",
        "Authorization",
        "content-type",
        "accept",
        "user-agent",
        "host",
        "forwarded",
        "x-forwarded-for",
        "via",
        "cookie",
        "set-cookie",
        "connection",
        "keep-alive",
        "x-kept",
    ];
    let mut given = Vec::new();
    while !c.is_empty() && given.len() < 32 {
        let k = if c.bool() { (*c.pick(&names)).to_string() } else { String::from_utf8_lossy(c.chunk()).into_owned() };
        given.push((k, String::from_utf8_lossy(c.chunk()).into_owned()));
    }
    let sent = request_headers(&given, "127.0.0.1:4790", "laptop");
    assert_eq!(sent.iter().filter(|(k, _)| k == "host").count(), 1);
    assert_eq!(sent.iter().filter(|(k, _)| k == "forwarded").collect::<Vec<_>>(), [&("forwarded".to_string(), "by=xenonet;for=laptop".to_string())]);
    assert!(sent.iter().all(|(k, _)| PASSED.contains(&k.as_str()) || k == "host" || k == "forwarded"), "{sent:?}");
    let back = answer_headers(&given);
    assert!(back.iter().all(|(k, _)| !matches!(k.as_str(), "set-cookie" | "connection" | "keep-alive")), "{back:?}");
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let shape = c.u8() % 6;
    match shape {
        0 => path(&String::from_utf8_lossy(c.rest())),
        1 | 2 => {
            let base = if shape == 1 {
                json!({ "schema": "network/svc/v2", "op": "request", "network": "net_00112233445566778899aabbccddeeff", "id": "svq_000102030405060708090a0b0c0d0e0f", "name": "files", "method": "GET", "path": "/sync/v1/x", "headers": [["authorization", "Bearer t"]], "length": 0 })
            } else {
                json!({ "schema": "network/svc/v2", "op": "answer", "status": 200, "headers": [["content-type", "text/plain"]], "length": 2 })
            };
            let v = if c.bool() {
                let over = c.value();
                overlay(base, over, &mut c)
            } else {
                c.value()
            };
            if shape == 1 {
                request(&v);
            } else {
                answer(&v);
            }
        }
        3 => target(&String::from_utf8_lossy(c.rest())),
        5 => {
            let titles_only = c.bool();
            let base = json!({ "schema": "network/svc/v2", "op": "window", "network": "net_00112233445566778899aabbccddeeff", "path": "/api/session/claude:abc/transcript?limit=120", "nonce": "AAAAAAAAAAAAAAAAAAAAAAAA", "at": 0, "cert": {}, "sig": "", "length": 0 });
            let over = c.value();
            window(&overlay(base, over, &mut c), titles_only);
        }
        _ => headers(&mut c),
    }
});
