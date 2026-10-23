#![no_main]

// Services through the network (docs/design/network.md, "Services
// through the network"): what a publishing host reads of another host's
// request before it passes it to a service, and what a caller's host reads
// of the answer. The first byte picks the input's shape:
//
// - 0: a path after /api/svc/<host>/<name>, normalized against an
//   allowlist.
// - 1: a request body (network/svc/v1), the schema's own with the input's
//   fields over it, or the input's JSON alone.
// - 2: an answer body, the same way.
// - 3: a published service's address.
// - 4: headers, as the caller and the service send them.
//
// Nothing may panic. A path that passes never holds what a server could
// read as another path, and normalizing it again changes nothing; the
// service is sent only the caller's four headers, its Host and one
// Forwarded; an answer never carries Set-Cookie or a hop-by-hop header;
// and an address that passes is this machine's, over http.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use xenonet_fuzz::net::{overlay, Choices};
use xenonet_net::svc::{answer_headers, normalize, request_headers, Answer, Published, Request, Target, PASSED};

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
    let Ok(r) = Request::parse(v) else { return };
    assert_eq!(Request::parse(&r.to_json()).as_ref(), Ok(&r), "a request reads back as another");
}

fn answer(v: &Value) {
    let Ok((id, a)) = Answer::parse(v) else { return };
    assert!((200..=599).contains(&a.status), "{a:?}");
    for (k, _) in &a.headers {
        assert!(!matches!(k.as_str(), "set-cookie" | "set-cookie2" | "connection" | "keep-alive" | "transfer-encoding" | "upgrade" | "te" | "trailer" | "content-length"), "{k}");
    }
    assert_eq!(answer_headers(&a.headers), a.headers, "cleaning twice changed it");
    let (id2, b) = Answer::parse(&a.to_json(&id)).expect("an answer's own JSON parses");
    assert_eq!((id2, b), (id, a), "an answer reads back as another");
}

fn target(text: &str) {
    let Ok(t) = Target::parse(text) else { return };
    assert!(t.ip.is_loopback(), "{t:?}");
    assert_eq!(Target::parse(&t.url()).as_ref(), Ok(&t), "an address reads back as another");
}

fn headers(c: &mut Choices<'_>) {
    let names = ["authorization", "Authorization", "content-type", "accept", "user-agent", "host", "forwarded", "x-forwarded-for", "via", "cookie", "set-cookie", "connection", "keep-alive", "x-kept"];
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
    let shape = c.u8() % 5;
    match shape {
        0 => path(&String::from_utf8_lossy(c.rest())),
        1 | 2 => {
            let base = if shape == 1 {
                json!({ "schema": "network/svc/v1", "op": "request", "id": "svq_000102030405060708090a0b0c0d0e0f", "name": "files", "method": "GET", "path": "/sync/v1/x", "headers": [["authorization", "Bearer t"]], "body": "" })
            } else {
                json!({ "schema": "network/svc/v1", "op": "answer", "id": "svq_000102030405060708090a0b0c0d0e0f", "status": 200, "headers": [["content-type", "text/plain"]], "body": "aGk" })
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
        _ => headers(&mut c),
    }
});
