#![no_main]

// The gateway's routes (/net/v1/…) as another machine reaches them, and
// what a host reads back. studio is the gateway, with its own files;
// laptop is a host that reads its answers. The first byte picks:
//
// - 0: a request as written on the wire ("METHOD target", headers, a
//   blank line, the body).
// - 1: a request signed by one of the fixed hosts (or with another's
//   key), its time and nonce from the input, then maybe one part changed:
//   the method, target, body, a signature header, or an Origin added.
// - 2: an up batch of frames built from the fixed keys, some changed.
// - 3: a join with a ticket the gateway keeps, maybe changed.
// - 4: the gateway's answers as laptop reads them: the signed clock
//   answer, the stream's hello, a removal, or bytes.
//
// Nothing may panic. A route other than join answers 2xx only to a
// request a member signed, within the minute; a signed request isn't taken
// twice; a changed frame is never accepted; a join succeeds only with the
// ticket's proof; laptop takes a time, a hello or a removal only as the
// gateway signed it.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use xenonet_fuzz::net::{self, Choices, Kept, LAPTOP, SPARE, STUDIO};
use xenonet_net::codec::b64;
use xenonet_net::frame::{Frame, HOST_KINDS, RELAY_BATCH_KINDS};
use xenonet_net::gateway::{join_body, NetBody, NetReply, NetRequest};
use xenonet_net::gwstore::TicketRow;
use xenonet_net::link::{first_line, refusal, Refusal};
use xenonet_net::request::{self, Signed, SKEW_MS};
use xenonet_net::roster::{HostRef, Op, Roster};
use xenonet_net::ticket;

static MACHINES: Mutex<(Kept, Kept)> = Mutex::new((Kept::new(STUDIO, 500), Kept::new(LAPTOP, 500)));

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a runtime"))
}

const AUTHED: [&str; 4] = ["/net/v1/link", "/net/v1/frames", "/net/v1/roster", "/net/v1/tickets"];
const SOURCES: [&str; 3] = ["203.0.113.7", "198.51.100.2", "2001:db8::1"];

struct Answer {
    status: u16,
    json: Value,
    reply: NetReply,
    /// The gateway's roster before the request, which a roster entry can
    /// change.
    before: Roster,
}

fn ask(gw: &net::Machine, req: NetRequest) -> Option<Answer> {
    let g = gw.gateway.clone()?;
    let before = gw.shared().roster().clone();
    let reply = runtime().block_on(async move { tokio::time::timeout(Duration::from_millis(60), g.handle(req)).await.ok() })?;
    Some(Answer { status: reply.status, json: reply.body_json(), reply, before })
}

/// A route other than join answers 2xx only to a request a member of the
/// gateway's roster signed over what it is, within the minute; a 410 only
/// to one a removed host's last keys signed.
fn only_signed(req: &NetRequest, a: &Answer) {
    let path = req.target.split('?').next().unwrap_or("");
    if !AUTHED.contains(&path) || req.header("origin").is_some() {
        assert!(!(a.status == 200 && req.header("origin").is_some()), "a request from a web page was answered");
        return;
    }
    if !(200..300).contains(&a.status) && a.status != 410 {
        return;
    }
    let get = |k: &str| req.header(k).map(str::to_string);
    let signed = Signed::from_headers(get).expect("answered a request with no signature");
    let v = a.before.view();
    let keys = if a.status == 410 { v.removal(&signed.host).map(|r| r.keys) } else { v.member(&signed.host).map(|m| m.keys) };
    let keys = keys.unwrap_or_else(|| panic!("answered {} to {}, which isn't in the roster", a.status, signed.host));
    assert!(signed.verify(&keys, net::NET, &req.method, &req.target, &req.body), "answered {} to a request whose signature doesn't check", a.status);
    if a.status != 410 {
        assert!((signed.time as f64 - net::now() as f64).abs() <= SKEW_MS, "answered a request a minute off");
    }
}

fn raw(gw: &net::Machine, data: &[u8]) {
    let (mut method, mut target, mut headers) = (String::new(), String::new(), Vec::new());
    let mut at = 0;
    for (n, line) in data.split(|&b| b == b'\n').enumerate() {
        at += line.len() + 1;
        let line = String::from_utf8_lossy(line.strip_suffix(b"\r").unwrap_or(line)).into_owned();
        if n == 0 {
            let (m, t) = line.split_once(' ').unwrap_or(("GET", &line));
            (method, target) = (m.to_string(), t.to_string());
        } else if line.is_empty() {
            break;
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let body = data.get(at..).unwrap_or_default().to_vec();
    // Where it came from, as the tunnel door reads it from the headers.
    let source = Some(xenonet_service::gateway::actor_gateway::ActorGateway::source_of(&headers));
    let req = NetRequest { method, target, headers, body, source };
    if let Some(a) = ask(gw, req.clone()) {
        only_signed(&req, &a);
    }
}

/// A b64url nonce from the input, or some other text when it says so.
fn nonce(c: &mut Choices<'_>) -> String {
    if c.rarely(24) {
        return String::from_utf8_lossy(c.chunk()).into_owned();
    }
    let mut b = [0u8; 16];
    b.copy_from_slice(&{
        let mut x = c.take(16).to_vec();
        x.resize(16, 0);
        x
    });
    // Unique within a run's requests, so only a repeat is a replay.
    b[15] = b[15].wrapping_add(fresh());
    b64(&b)
}

fn fresh() -> u8 {
    static N: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// A body for a route, from the fixed network, and whether it's a relay
/// batch.
fn body_for(gw: &net::Machine, path: &str, signer: usize, c: &mut Choices<'_>) -> (Vec<u8>, bool) {
    let body = match path {
        "/net/v1/frames" => {
            let b = batch(signer, c);
            return (b.body.into_bytes(), b.relay);
        }
        "/net/v1/roster" => {
            let roster = gw.shared().roster().clone();
            let op = *c.pick(&[Op::Add, Op::Remove, Op::Window, Op::Gateway]);
            let host = c.below(net::HOSTS.len());
            let gateway = (op == Op::Gateway).then(|| xenonet_net::roster::GatewayAt { address: "gw2.example.net".into(), version: roster.view().gateway_version + 1 });
            let mut e = roster.draft(op, HostRef { id: net::HOSTS[host].into(), keys: net::public(host), window: c.bool() }, gateway, net::now());
            for _ in 0..=c.below(3) {
                let s = c.below(net::HOSTS.len());
                e.sign_by(net::HOSTS[s], net::key(s));
            }
            json!({ "entry": e.to_json(), "propose": c.bool() }).to_string().into_bytes()
        }
        "/net/v1/tickets" => json!({ "hash": ticket::hash_b64(net::TICKET), "host": *c.pick(&["spare", "laptop", "Bad"]), "window": c.bool() }).to_string().into_bytes(),
        _ => c.chunk().to_vec(),
    };
    (body, false)
}

/// An up batch from `signer`.
fn batch(signer: usize, c: &mut Choices<'_>) -> Batch {
    let mut lines = Vec::new();
    let mut ok = Vec::new();
    let relay = c.rarely(16);
    for _ in 0..=c.below(6) {
        let from = if c.rarely(32) { c.below(net::HOSTS.len()) } else { signer };
        let to = c.below(net::HOSTS.len());
        let kind = if relay && c.bool() { *c.pick(&RELAY_BATCH_KINDS) } else { *c.pick(&HOST_KINDS) };
        let key = if c.rarely(24) { c.below(net::HOSTS.len()) } else { from };
        let at = net::now() + i64::from(c.i16());
        let id = net::fresh_frame_id(c.u16());
        let body = net::body_for(kind, c);
        let f = net::frame(id, kind, net::HOSTS[from], net::HOSTS[to], at, &body, key, to, c.u8());
        let line = match c.below(4) {
            1 => {
                let pick = c.u8();
                net::change_field(&f, pick, c.value())
            }
            2 => String::from_utf8_lossy(&net::change_bytes(f.line().trim_end(), c)).into_owned(),
            _ => f.line().trim_end().to_string(),
        };
        // A line that isn't JSON would spoil the whole batch.
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let changed = Frame::parse(&v).map_or(true, |g| !net::same_frame(&f, &g));
        let member = |h: &str| net::roster().view().member(h).is_some();
        let acceptable = !changed && key == from && from == signer && member(&f.to) && f.to != f.from && RELAY_BATCH_KINDS.contains(&kind) == relay;
        if acceptable {
            ok.push(f.id.clone());
        }
        lines.push(v.to_string());
    }
    let ack = if c.bool() { json!({ "epoch": "x", "seq": c.u16() }) } else { Value::Null };
    Batch { body: format!("{{\"frames\":[{}],\"ack\":{ack}}}", lines.join(",")), ok, relay }
}

/// An up batch: its body, the ids the gateway may accept, and whether
/// it's a relay batch (`?relay=1`).
struct Batch {
    body: String,
    ok: Vec<String>,
    relay: bool,
}

/// A request from `signer`, signed with `key`'s key, at `time`, then
/// changed as `change` says. The request, and whether it's as signed.
#[expect(clippy::too_many_arguments, reason = "each is a part of the request a run picks")]
fn signed(method: &str, target: &str, body: Vec<u8>, signer: usize, key: usize, time: i64, nonce: &str, change: u8, c: &mut Choices<'_>) -> (NetRequest, bool) {
    let headers = request::sign(net::key(key), net::NET, net::HOSTS[signer], method, target, time, nonce, &body);
    let mut req = NetRequest { method: method.into(), target: target.into(), headers, body, source: Some(SOURCES[c.below(3)].into()) };
    let mut intact = true;
    match change {
        0 => req.method = if method == "GET" { "POST".into() } else { "GET".into() },
        1 => req.target.push_str(&String::from_utf8_lossy(c.chunk())),
        2 => {
            if req.body.is_empty() {
                req.body.push(b' ');
            } else {
                let i = usize::from(c.u16()) % req.body.len();
                req.body[i] ^= 1 | c.u8();
            }
        }
        3..=6 => {
            let name = [request::HOST, request::TIME, request::NONCE, request::SIG][usize::from(change - 3)];
            let v = String::from_utf8_lossy(c.chunk()).into_owned();
            if let Some(h) = req.headers.iter_mut().find(|(k, _)| k == name) {
                h.1 = v;
            }
        }
        7 => {
            let i = c.below(req.headers.len());
            req.headers.remove(i);
        }
        8 => req.headers.push(("origin".into(), "https://example.com".into())),
        _ => return (req, true),
    }
    // A change can leave the request as it was (a header set to its own
    // value): it's still as signed then.
    let get = |k: &str| req.header(k).map(str::to_string);
    if !Signed::from_headers(get).is_some_and(|s| s.verify(&net::public(key), net::NET, &req.method, &req.target, &req.body) && s.host == net::HOSTS[signer] && s.time == time)
        || req.header("origin").is_some()
    {
        intact = false;
    }
    (req, intact)
}

fn route(c: &mut Choices<'_>) -> (&'static str, String) {
    let (method, path) = *c.pick(&[
        ("GET", "/net/v1/link"),
        ("POST", "/net/v1/frames"),
        ("GET", "/net/v1/roster"),
        ("POST", "/net/v1/roster"),
        ("POST", "/net/v1/tickets"),
        ("GET", "/net/v1/frames"),
        ("PUT", "/net/v1/roster"),
    ]);
    let query = match (method, path) {
        ("GET", "/net/v1/link") => format!("?epoch={}&after={}{}", *c.pick(&["", "x"]), c.u8(), if c.rarely(32) { "&wait=1" } else { "" }),
        ("GET", "/net/v1/roster") => format!("?from={}", c.u8() % 8),
        _ if c.rarely(16) => format!("?{}", String::from_utf8_lossy(c.chunk())),
        _ => String::new(),
    };
    (method, format!("{path}{query}"))
}

fn signed_request(gw: &net::Machine, laptop: &net::Machine, c: &mut Choices<'_>) {
    let signer = c.below(net::HOSTS.len());
    let key = if c.rarely(24) { c.below(net::HOSTS.len()) } else { signer };
    let (method, mut target) = route(c);
    let (body, relay) = body_for(gw, target.split('?').next().unwrap_or(""), signer, c);
    if relay {
        target.push_str(if target.contains('?') { "&relay=1" } else { "?relay=1" });
    }
    let skew = i64::from(c.i16()) * 4;
    let time = net::now() + skew;
    let n = nonce(c);
    // Most runs change nothing; a short input changes nothing.
    let change = (c.u8() % 16 + 9) % 16;
    let (req, intact) = signed(method, &target, body, signer, key, time, &n, change, c);
    let Some(a) = ask(gw, req.clone()) else { return };
    only_signed(&req, &a);
    let member = net::roster().view().member(net::HOSTS[signer]).is_some();
    let authed = intact && key == signer && member && (skew as f64).abs() <= SKEW_MS && Signed::from_headers(|k| req.header(k).map(str::to_string)).is_some();
    if !authed {
        assert!(!(200..300).contains(&a.status), "answered {} to a request that isn't as a member signed it: {:?}", a.status, a.json);
    }
    // The gateway's time, signed for this request: laptop takes it.
    let path = target.split('?').next().unwrap_or("");
    let known = matches!((method, path), ("GET", "/net/v1/link" | "/net/v1/roster") | ("POST", "/net/v1/frames" | "/net/v1/roster" | "/net/v1/tickets"));
    if known && intact && key == signer && signer == LAPTOP && (skew as f64).abs() > SKEW_MS && Signed::from_headers(|k| req.header(k).map(str::to_string)).is_some() {
        assert_eq!((a.status, a.json["code"].as_str()), (401, Some("clock")), "{:?}", a.json);
        match refusal(laptop.shared(), &a.reply, &n) {
            Refusal::Clock(now) => assert_eq!(now as i64, net::now(), "laptop took another time than the gateway's"),
            _ => panic!("laptop didn't take the gateway's signed time"),
        }
        assert!(!matches!(refusal(laptop.shared(), &a.reply, "bm90LXRoaXMtcmVxdWVzdA"), Refusal::Clock(_)), "laptop took a time signed for another request");
    }
    // The same request again is a replay, once the first was taken, and
    // didn't change the roster: a host that removed itself is answered
    // that it's removed, and a POST to a gateway that moved is turned away
    // before its signature is read.
    if authed && AUTHED.contains(&path) && a.status != 404 && a.json["code"] != "moved" && gw.shared().roster().len() == a.before.len() {
        if let Some(again) = ask(gw, req.clone()) {
            assert_eq!((again.status, again.json["code"].as_str()), (409, Some("replay")), "a signed request was taken twice");
        }
    }
}

fn frames(gw: &net::Machine, c: &mut Choices<'_>) {
    let signer = *c.pick(&[STUDIO, net::DESK, LAPTOP, LAPTOP, net::GONE]);
    let b = batch(signer, c);
    let target = if b.relay { "/net/v1/frames?relay=1" } else { "/net/v1/frames" };
    let n = nonce(c);
    let (req, _) = signed("POST", target, b.body.into_bytes(), signer, signer, net::now(), &n, 255, c);
    let Some(a) = ask(gw, req.clone()) else { return };
    only_signed(&req, &a);
    for id in a.json["accepted"].as_array().into_iter().flatten() {
        let id = id.as_str().unwrap_or("");
        assert!(b.ok.iter().any(|x| x == id), "the gateway accepted a frame it shouldn't have: {id} in {:?}", a.json);
    }
}

fn join(gw: &net::Machine, c: &mut Choices<'_>) {
    let g = gw.gateway.as_ref().expect("studio's gateway");
    let now = net::now();
    let host = *c.pick(&["spare", "spare", "laptop"]);
    let maker = *c.pick(&["studio", "studio", "desk"]);
    let window = c.bool();
    let _ =
        g.add_ticket(&TicketRow { hash: ticket::hash_b64(net::TICKET), host: host.into(), window, maker: maker.into(), made: now, expires: now + ticket::TICKET_MS as i64 }, now);
    let joiner = if c.rarely(32) { c.below(net::HOSTS.len()) } else { SPARE };
    let code = if c.rarely(32) { ticket::normalize(&String::from_utf8_lossy(c.chunk())).unwrap_or_else(|| "KX7Q49MZT3".into()) } else { net::TICKET.into() };
    let n = nonce(c);
    let mut body = join_body(net::key(joiner), &code, &n, "linux", "0.6.1", c.bool());
    const FIELDS: [&str; 10] = ["sign", "seal", "nonce", "proof", "makerProof", "tag", "platform", "version", "desktop", "sig"];
    let mut proved = code == net::TICKET && n.len() >= 16;
    match c.below(4) {
        1 => {
            let f = *c.pick(&FIELDS);
            let v = c.value();
            if body[f] != v {
                if matches!(f, "sign" | "seal" | "nonce" | "proof" | "sig") {
                    proved = false;
                }
                body[f] = v;
            }
        }
        2 => {
            body = c.value();
            proved = false;
        }
        _ => {}
    }
    let req =
        NetRequest { method: "POST".into(), target: "/net/v1/join".into(), headers: Vec::new(), body: body.to_string().into_bytes(), source: Some(SOURCES[c.below(3)].into()) };
    let Some(a) = ask(gw, req) else { return };
    let keys = xenonet_net::keys::PublicKeys::from_b64(body["sign"].as_str().unwrap_or(""), body["seal"].as_str().unwrap_or(""));
    let roster = gw.shared().roster().clone();
    let holds = keys.is_some_and(|k| roster.view().member(host).is_some_and(|m| m.keys == k));
    if a.status == 200 {
        assert!(proved, "a join without the ticket's proof succeeded: {body}");
        assert!(holds, "a join succeeded and the roster doesn't hold its host");
        let gw_keys = net::public(STUDIO);
        assert!(ticket::check_gateway(&ticket::hash(net::TICKET), &gw_keys.key_id(), &n, a.json["proof"].as_str().unwrap_or("")), "the join's answer doesn't prove the ticket");
    }
    if !proved && host == "spare" {
        assert!(!holds || joiner == SPARE && keys == Some(net::public(SPARE)) && a.status == 200, "a join without the ticket's proof put {host} in the roster");
    }
}

/// The gateway's answers, as laptop reads them.
fn answers(gw: &net::Machine, laptop: &net::Machine, c: &mut Choices<'_>) {
    let s = laptop.shared();
    match c.below(4) {
        // A clock answer for laptop's request, maybe changed.
        0 => {
            let n = nonce(c);
            let time = net::now() + 120_000 + i64::from(c.u16());
            let (req, _) = signed("GET", "/net/v1/roster?from=0", Vec::new(), LAPTOP, LAPTOP, time, &n, 255, c);
            let Some(a) = ask(gw, req) else { return };
            let Some(line) = a.json["frame"].as_str().map(str::to_string) else { return };
            let f = Frame::parse_line(&line).expect("the gateway's own frame");
            let (changed, same) = change(&f, c);
            let mut v = a.json.clone();
            v["frame"] = json!(changed);
            let r = NetReply::json(401, &v);
            if let Refusal::Clock(now) = refusal(s, &r, &n) {
                assert!(same, "laptop took a time from a changed answer: {changed}");
                assert_eq!(now as i64, net::now());
            } else {
                assert!(!same, "laptop didn't take the gateway's signed time");
            }
        }
        // The stream's hello, maybe changed.
        1 => {
            let n = nonce(c);
            let (req, _) = signed("GET", "/net/v1/link?epoch=&after=0", Vec::new(), LAPTOP, LAPTOP, net::now(), &n, 255, c);
            let Some(a) = ask(gw, req) else { return };
            let Some(line) = runtime().block_on(first_line(a.reply.body, Duration::from_millis(40))) else { return };
            let Ok(f) = Frame::parse_line(&line) else { return };
            let (changed, same) = change(&f, c);
            match Frame::parse_line(&changed).map(|g| s.hello(&g, &n)) {
                Ok(Ok(_)) => assert!(same, "laptop took a changed hello: {changed}"),
                _ => assert!(!same, "laptop refused the gateway's own hello: {line}"),
            }
        }
        // A removal, maybe changed.
        2 => {
            let body = json!({ "network": net::NET, "host": "laptop", "by": ["studio"], "at": net::now(), "n": 5 });
            let f = net::frame(net::fresh_frame_id(c.u16()), "removed", "studio", "laptop", net::now(), &body, STUDIO, LAPTOP, 0);
            let (changed, same) = change(&f, c);
            let r = NetReply { status: 410, headers: Vec::new(), body: NetBody::Full(changed.clone().into_bytes()) };
            match refusal(s, &r, "bm9uY2U") {
                Refusal::Removed => assert!(same, "laptop took a changed removal: {changed}"),
                _ => assert!(!same || s.is_removed(), "laptop refused the gateway's own removal"),
            }
        }
        // Bytes.
        _ => {
            let status = *c.pick(&[200, 401, 409, 410, 500]);
            let bytes = c.rest().to_vec();
            let _ = refusal(s, &NetReply { status, headers: Vec::new(), body: NetBody::Full(bytes.clone()) }, "bm9uY2Utb2YtdGhlLWxpbms");
            if let Some(line) = runtime().block_on(first_line(NetBody::Full(bytes), Duration::from_millis(5))) {
                if let Ok(f) = Frame::parse_line(&line) {
                    let _ = s.hello(&f, "bm9uY2Utb2YtdGhlLWxpbms");
                    let _ = s.frame(&f);
                }
            }
        }
    }
}

/// A frame's line, changed as the input says (or not): the line, and
/// whether it still says what the frame did.
fn change(f: &Frame, c: &mut Choices<'_>) -> (String, bool) {
    let line = match c.below(3) {
        0 => f.line().trim_end().to_string(),
        1 => {
            let pick = c.u8();
            net::change_field(f, pick, c.value())
        }
        _ => String::from_utf8_lossy(&net::change_bytes(f.line().trim_end(), c)).into_owned(),
    };
    let same = Frame::parse_line(&line).is_ok_and(|g| net::same_frame(f, &g));
    (line, same)
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let mode = c.u8();
    let mut kept = MACHINES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // An hour on, each run: nonces, rates, bad tickets and tickets lapse.
    net::clock().fetch_add(3_600_000, Ordering::Relaxed);
    let (gw_kept, laptop_kept) = &mut *kept;
    let (gw, laptop) = (gw_kept.get(), laptop_kept.get());
    match mode % 5 {
        0 => raw(gw, c.rest()),
        1 => signed_request(gw, laptop, &mut c),
        2 => frames(gw, &mut c),
        3 => join(gw, &mut c),
        _ => answers(gw, laptop, &mut c),
    }
    if Kept::changed(gw) || Kept::changed(laptop) {
        gw_kept.spoilt();
        laptop_kept.spoilt();
    }
});
