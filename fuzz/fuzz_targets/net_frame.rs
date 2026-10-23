#![no_main]

// A frame on the network's wire (network/frame/v1), as a host reads one
// from its gateway's stream: parsed, its head and signature checked
// against the roster, its body opened, and then what laptop's node does
// with it. The first byte picks the input's shape:
//
// - 0: a whole line, as it came.
// - 1: a frame built from the fixed network's keys, then one field of its
//   JSON changed.
// - 2: the same frame, then bytes of its line changed.
// - 3: a gateway frame (hello, ping, gap, roster, removed) from studio,
//   maybe changed, read as the stream's hello, a clock answer, a removal
//   and a frame after the hello.
//
// Nothing may panic, and a changed frame never checks or opens: the node
// passes on only what its sender signed and sealed for it.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use xenonet_fuzz::net::{self, body_for, Choices, Kept, LAPTOP, STUDIO};
use xenonet_net::frame::{open_frame, Frame, FRAME_KINDS, GATEWAY_KINDS, HOST_KINDS};
use xenonet_net::gateway::{NetBody, NetReply};
use xenonet_net::link::{refusal, Refusal};

static LAPTOP_NODE: Mutex<Kept> = Mutex::new(Kept::new(LAPTOP, 5000));
const NONCE: &str = "bm9uY2Utb2YtdGhlLWxpbms";

/// What laptop's node passed on must be a frame its sender signed, for
/// laptop, sealed for laptop's key, with the body it opens to.
fn passed_on_rightly(m: &net::Machine, f: &Frame) {
    let roster = m.shared().roster().clone();
    for got in m.caught.take() {
        let keys = roster.view().member(&got.from).map(|x| x.keys);
        assert!(keys.is_some_and(|k| f.verify(net::NET, &k)), "passed on a frame its sender didn't sign: {f:?}");
        assert_eq!(f.to, "laptop", "passed on a frame for another host");
        assert!(HOST_KINDS.contains(&f.kind.as_str()) && f.sealed, "passed on a {} frame as a body", f.kind);
        let body = open_frame(net::NET, net::key(LAPTOP), f).expect("passed on a frame that doesn't open");
        assert_eq!(got.body, body, "passed on another body than the frame's");
        assert_eq!(got.from_window_host, roster.view().is_window(&got.from));
    }
}

/// What the parse gives back when its line is written again.
fn round_trips(f: &Frame) {
    let again = Frame::parse_line(&f.line()).expect("a frame's own line parses");
    assert_eq!(&again, f, "a frame's line reads back as another frame");
}

fn whole(m: &net::Machine, line: &[u8]) {
    let text = String::from_utf8_lossy(line);
    let Ok(f) = Frame::parse_line(&text) else { return };
    round_trips(&f);
    for i in 0..net::HOSTS.len() {
        let _ = f.verify(net::NET, &net::public(i));
        let _ = open_frame(net::NET, net::key(i), &f);
    }
    let _ = f.body_json();
    let _ = m.shared().frame(&f);
    passed_on_rightly(m, &f);
}

/// A frame of the fixed network: its kind, sender, recipient, time and
/// body from the choices, signed by its sender (or, sometimes, another
/// host's key) and sealed for its recipient (or, sometimes, another).
/// The frame, and whether it's what its sender would have sent.
fn built(c: &mut Choices<'_>, gateway_only: bool) -> (Frame, bool) {
    let kind = if gateway_only { *c.pick(&GATEWAY_KINDS) } else { *c.pick(&FRAME_KINDS) };
    let from = if gateway_only { STUDIO } else { c.below(net::HOSTS.len()) };
    let to = *c.pick(&["laptop", "laptop", "laptop", "desk", "studio", "*"]);
    let at = net::now() + i64::from(c.i16()) * 37;
    let controls = c.u8();
    let signer = if controls & 0x0f == 0x0f { (from + 1 + c.below(net::HOSTS.len() - 1)) % net::HOSTS.len() } else { from };
    let seal_for = if controls & 0xf0 == 0xf0 { c.below(net::HOSTS.len()) } else { net::index_of(to).unwrap_or(LAPTOP) };
    let eph = c.u8();
    let id = net::fresh_frame_id(c.u16());
    let body = body_for(kind, c);
    let f = net::frame(id, kind, net::HOSTS[from], to, at, &body, signer, seal_for, eph);
    (f, signer == from)
}

/// `g` is `f` after a change. A change a signature doesn't cover leaves it
/// as it was; any other never checks against its sender's key, and one
/// to what sealing binds never opens.
fn changed_never_checks(f: &Frame, honest: bool, g: &Frame) {
    let roster = net::roster();
    let same = net::same_frame(f, g);
    if same && honest {
        return;
    }
    if let Some(k) = net::keys_in(&roster, &g.from) {
        assert!(!g.verify(net::NET, &k), "a changed frame checks against {}'s key:\nwas {f:?}\nnow {g:?}", g.from);
    }
    if g.from == "spare" {
        assert!(!g.verify(net::NET, &net::public(net::SPARE)), "a changed frame checks: {g:?}");
    }
    if net::sealing_differs(f, g) {
        for i in 0..net::HOSTS.len() {
            assert!(open_frame(net::NET, net::key(i), g).is_err(), "a changed frame opens for {}:\nwas {f:?}\nnow {g:?}", net::HOSTS[i]);
        }
    }
}

fn mutated(m: &net::Machine, c: &mut Choices<'_>, by_field: bool) {
    let (f, honest) = built(c, false);
    let line = if !c.bool() {
        // As it was sent, now and then, so frames reach the node whole.
        f.line().trim_end().as_bytes().to_vec()
    } else if by_field {
        let pick = c.u8();
        net::change_field(&f, pick, c.value()).into_bytes()
    } else {
        net::change_bytes(f.line().trim_end(), c)
    };
    let Ok(g) = Frame::parse_line(&String::from_utf8_lossy(&line)) else {
        // Bytes that aren't a frame: the node never sees them.
        return;
    };
    round_trips(&g);
    changed_never_checks(&f, honest, &g);
    let _ = m.shared().frame(&g);
    let passed = m.caught.incoming.lock().map(|l| l.len()).unwrap_or(0);
    if passed > 0 {
        assert!(honest && net::same_frame(&f, &g), "the node passed on a changed frame:\nwas {f:?}\nnow {g:?}");
    }
    passed_on_rightly(m, &g);
}

/// A gateway frame read as each thing a host takes from its gateway.
fn from_gateway(m: &net::Machine, c: &mut Choices<'_>) {
    let (f, honest) = built(c, true);
    let line = match c.below(3) {
        0 => f.line().trim_end().as_bytes().to_vec(),
        1 => {
            let pick = c.u8();
            net::change_field(&f, pick, c.value()).into_bytes()
        }
        _ => net::change_bytes(f.line().trim_end(), c),
    };
    let text = String::from_utf8_lossy(&line).into_owned();
    let Ok(g) = Frame::parse_line(&text) else {
        // The refusal's reader takes any bytes.
        let _ = refusal(m.shared(), &NetReply { status: 410, headers: Vec::new(), body: NetBody::Full(line) }, NONCE);
        return;
    };
    round_trips(&g);
    changed_never_checks(&f, honest, &g);
    let genuine = honest && net::same_frame(&f, &g) && g.from == "studio";
    let body = g.body_json().unwrap_or(Value::Null);
    let s = m.shared();
    if s.hello(&g, NONCE).is_ok() {
        assert!(genuine && g.kind == "hello" && body["nonce"] == NONCE && body["network"] == net::NET, "took a hello it shouldn't have: {g:?}");
    }
    if let Ok(now) = s.clock_answer(&json!({ "frame": text }), NONCE) {
        assert!(genuine && g.kind == "ping" && g.to == "laptop" && body["nonce"] == NONCE, "took a time it shouldn't have: {g:?}");
        assert_eq!(Some(now), body["now"].as_f64());
    }
    match refusal(s, &NetReply::json(401, &json!({ "code": "clock", "frame": text })), NONCE) {
        Refusal::Clock(_) => assert!(genuine && g.kind == "ping", "a refusal's time that didn't check was taken: {g:?}"),
        Refusal::Removed | Refusal::Other(_) => {}
    }
    let was_removed = s.is_removed();
    let gaps = m.caught.gaps.load(Ordering::Relaxed);
    let _ = refusal(s, &NetReply { status: 410, headers: Vec::new(), body: NetBody::Full(line) }, NONCE);
    let _ = s.frame(&g);
    if s.is_removed() && !was_removed {
        assert!(genuine && g.kind == "removed" && g.to == "laptop" && body["host"] == "laptop", "a removal that didn't check was taken: {g:?}");
    }
    if m.caught.gaps.load(Ordering::Relaxed) > gaps {
        // A gap, or a hello with a new epoch.
        assert!(genuine && matches!(g.kind.as_str(), "gap" | "hello"), "a gap that didn't check was taken: {g:?}");
    }
    passed_on_rightly(m, &g);
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let mode = c.u8();
    let mut kept = LAPTOP_NODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // A minute on, each run: operator nonces and frame times move with it.
    net::clock().fetch_add(60_000, Ordering::Relaxed);
    let m = kept.get();
    match mode % 4 {
        0 => whole(m, c.rest()),
        1 => mutated(m, &mut c, true),
        2 => mutated(m, &mut c, false),
        _ => from_gateway(m, &mut c),
    }
    let _ = m.caught.take();
    if Kept::changed(m) {
        kept.spoilt();
    }
});
