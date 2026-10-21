// Known answers for the network's crypto (test/fixtures/net/known-answers.json):
// fixed keys, a fixed ephemeral key and fixed ids, so another
// implementation can check its key ids, heads, signatures, sealing, ticket
// proofs, request signatures and roster hashes against these, byte for
// byte. Ed25519 signs deterministically, so all of it is reproducible.
//
// XENONET_WRITE_FIXTURES=1 cargo test -p xenonet-net --test known_answers
// writes the file again.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use xenonet_net::codec::{b64, unb64_n};
use xenonet_net::frame::{gateway_frame, host_frame, open_frame, Draft, Frame};
use xenonet_net::keys::HostKey;
use xenonet_net::roster::{found, HostRef, Op, Roster};
use xenonet_net::{request, ticket};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/net/known-answers.json");
const NET: &str = "net_00112233445566778899aabbccddeeff";

fn secret(byte: u8) -> [u8; 32] {
    let mut s = [byte; 32];
    s[0] = 0x40 | (byte & 0x0f);
    s
}

fn host(name: &str, sign: u8, seal: u8) -> (HostKey, Value) {
    let k = HostKey::from_secrets(&secret(sign), secret(seal)).unwrap();
    let v = json!({
        "id": name,
        "signSeed": b64(&secret(sign)),
        "sealSecret": b64(&secret(seal)),
        "sign": k.public().sign_b64(),
        "seal": k.public().seal_b64(),
        "keyId": k.key_id(),
    });
    (k, v)
}

fn answers() -> Value {
    let (studio, studio_v) = host("studio", 1, 2);
    let (laptop, laptop_v) = host("laptop", 3, 4);
    let body = json!({ "schema": "network/message/v1", "mid": "msg_00000000-0000-4000-8000-000000000001", "body": "hello from studio" });
    let d = Draft { network: NET, id: "frm_00000000-0000-4000-8000-0000000000aa".into(), kind: "message", from: "studio", to: "laptop", at: 1_792_512_345_678 };
    let f = host_frame(&d, &studio, laptop.public(), &body, Some(secret(9))).unwrap();
    assert_eq!(open_frame(NET, &laptop, &f).unwrap(), body);
    let g = gateway_frame(
        &Draft { kind: "ping", from: "studio", to: "laptop", id: "frm_00000000-0000-4000-8000-0000000000bb".into(), ..d },
        &studio,
        &json!({ "now": 1_792_512_345_678_i64, "linked": ["laptop", "studio"] }),
    );
    let code = "KX7Q49MZT2";
    let hash = ticket::hash(code);
    let nonce = "bm9uY2Utb2YtdGhlLWpvaW4";
    let mut r = Roster::new();
    r.offer(found(NET, "studio", &studio, None, 1_792_512_000_000)).unwrap();
    let mut add = r.draft(Op::Add, HostRef { id: "laptop".into(), keys: *laptop.public(), window: false }, None, 1_792_512_100_000);
    add.sign_by("studio", &studio);
    r.offer(add).unwrap();
    let req_body = br#"{"frames":[]}"#;
    let target = "/net/v1/frames";
    let headers = request::sign(&laptop, NET, "laptop", "POST", target, 1_792_512_345_000, "cmVxdWVzdC1ub25jZQ", req_body);
    json!({
        "about": "Known answers for one network across machines (docs/design/network.md, section 3): written by crates/xenonet-net/tests/known_answers.rs. Secrets here are test values, never a real host's.",
        "network": NET,
        "hosts": [studio_v, laptop_v],
        "frame": {
            "ephemeralSecret": b64(&secret(9)),
            "body": body,
            "sealingHead": f.sealing_head(NET),
            "head": f.head(NET),
            "frame": f.to_json(),
        },
        "gatewayFrame": { "head": g.head(NET), "frame": g.to_json() },
        "ticket": {
            "code": code,
            "display": ticket::display(code),
            "hash": b64(&hash),
            "nonce": nonce,
            "joinerKeyId": laptop.key_id(),
            "joinProof": ticket::join_proof(&hash, nonce, &laptop.key_id()),
            "gatewayKeyId": studio.key_id(),
            "gatewayProof": ticket::gateway_proof(&hash, &studio.key_id(), nonce),
        },
        "request": {
            "method": "POST",
            "target": target,
            "body": String::from_utf8_lossy(req_body),
            "signingString": request::signing_string(NET, "POST", target, 1_792_512_345_000, "cmVxdWVzdC1ub25jZQ", "laptop", req_body),
            "headers": headers.into_iter().map(|(k, v)| (k, Value::String(v))).collect::<serde_json::Map<_, _>>(),
        },
        "roster": r.to_json(),
        "rosterHead": r.head(),
    })
}

#[test]
fn the_crypto_gives_its_known_answers() {
    let got = answers();
    if std::env::var("XENONET_WRITE_FIXTURES").as_deref() == Ok("1") {
        std::fs::create_dir_all(std::path::Path::new(FILE).parent().unwrap()).unwrap();
        std::fs::write(FILE, format!("{}\n", serde_json::to_string_pretty(&got).unwrap())).unwrap();
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    assert_eq!(got, want, "the known answers changed: if that's meant, write them again (see the top of this file)");
}

#[test]
fn the_fixtures_check_from_their_own_values() {
    // What another implementation would do: read the file, rebuild the
    // keys from their secrets, and check each answer.
    let v: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    let net = v["network"].as_str().unwrap();
    let key = |h: &Value| HostKey::from_secrets(&unb64_n::<32>(h["signSeed"].as_str().unwrap()).unwrap(), unb64_n::<32>(h["sealSecret"].as_str().unwrap()).unwrap()).unwrap();
    let studio = key(&v["hosts"][0]);
    let laptop = key(&v["hosts"][1]);
    assert_eq!(studio.key_id(), v["hosts"][0]["keyId"]);
    let f = Frame::parse(&v["frame"]["frame"]).unwrap();
    assert_eq!(f.head(net), v["frame"]["head"]);
    assert!(f.verify(net, studio.public()));
    assert_eq!(open_frame(net, &laptop, &f).unwrap(), v["frame"]["body"]);
    let g = Frame::parse(&v["gatewayFrame"]["frame"]).unwrap();
    assert!(g.verify(net, studio.public()));
    let t = &v["ticket"];
    let hash = ticket::hash(t["code"].as_str().unwrap());
    assert!(ticket::check_join(&hash, t["nonce"].as_str().unwrap(), t["joinerKeyId"].as_str().unwrap(), t["joinProof"].as_str().unwrap()));
    assert!(ticket::check_gateway(&hash, t["gatewayKeyId"].as_str().unwrap(), t["nonce"].as_str().unwrap(), t["gatewayProof"].as_str().unwrap()));
    let r = Roster::from_json(&v["roster"]).unwrap();
    assert_eq!(r.head().as_deref(), v["rosterHead"].as_str());
    let q = &v["request"];
    let h = |k: &str| q["headers"][k].as_str().map(str::to_string);
    let signed = request::Signed::from_headers(h).unwrap();
    assert!(signed.verify(laptop.public(), net, "POST", q["target"].as_str().unwrap(), q["body"].as_str().unwrap().as_bytes()));
}
