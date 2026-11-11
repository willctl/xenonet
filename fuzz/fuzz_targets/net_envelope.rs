#![no_main]

// An envelope (xenonet-net/2), as a machine reads one off its link:
// decoded, then opened for laptop with laptop's transport key and checked
// against the fixed network's chain, or, live, taken from one host's
// connection. The first byte picks the input's shape:
//
// - 0: the bytes as they came.
// - 1: a sealed envelope built from the fixed keys (a feed entry by any
//   host, sometimes signed by another host's machine key, sealed for any
//   host and named for any), then one field of its JSON changed.
// - 2: the same envelope, then bytes of its JSON changed.
// - 3: a live envelope from any host, maybe changed, read from any host's
//   connection.
//
// Nothing may panic. What opens for laptop is exactly what its author
// signed and sealed for laptop: an envelope named for laptop, from a
// member, whose head says what the entry inside says. A live envelope is
// taken only from its author's own connection. An envelope taken once is
// refused the second time.

use libfuzzer_sys::fuzz_target;
use std::sync::Mutex;
use xenonet_fuzz::net::{self, Choices, LAPTOP};
use xenonet_id::sign::{Purpose, Signer};
use xenonet_log::feed::{FeedEntry, Kind, Verified};
use xenonet_net::envelope::{Class, Envelope, Me, Seen, To};
use xenonet_net::roster::View;

static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

fn view() -> View {
    net::roster().view()
}

/// Opens `env` for laptop.
fn open_for_laptop(env: &Envelope, view: &View) -> Option<Verified> {
    let id = net::machine(LAPTOP).id();
    env.open_entry(&Me { machine: &id, key: net::key(LAPTOP) }, view).ok()
}

/// What laptop took must be its author's, for laptop, as the head says;
/// and taken once.
fn took_rightly(env: &Envelope, got: &Verified, view: &View) {
    let author = view.by_machine(&got.author).expect("took an entry from outside the network");
    assert!(got.entry().clone().verify(&author.machine).is_ok(), "took an entry its author didn't sign");
    assert_eq!(env.to, To::Machine(net::machine(LAPTOP).id()), "took an envelope for another machine");
    assert_eq!((env.network.as_str(), env.id.as_str(), env.kind.as_str(), env.author.as_str()), (got.network.as_str(), got.id().as_str(), got.kind.as_str(), got.author.as_str()));
    assert_eq!((env.pos, env.at), (Some(got.pos()), got.at), "the head's place or time isn't the entry's");
    let mut seen = SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let seen = seen.get_or_insert_with(|| Seen::new(4096));
    if seen.take(env).is_ok() {
        assert!(seen.take(env).is_err(), "an envelope was taken twice");
    }
}

fn round_trips(env: &Envelope) {
    let again = Envelope::decode(&env.encode()).expect("an envelope's own JSON reads");
    assert_eq!(&again, env, "an envelope's JSON reads back as another");
}

fn whole(data: &[u8]) {
    let Ok(env) = Envelope::decode(data) else { return };
    round_trips(&env);
    let v = view();
    if let Some(got) = open_for_laptop(&env, &v) {
        took_rightly(&env, &got, &v);
    }
    for i in 0..net::HOSTS.len() {
        let id = net::machine(i).id();
        let _ = env.open_entry(&Me { machine: &id, key: net::key(i) }, &v);
        if env.open_live(&net::public(i).sign, &v).is_ok() {
            assert!(v.by_machine(&env.author).is_some_and(|m| m.keys == net::public(i)), "took a live envelope from another's connection");
        }
    }
}

/// An entry of host `author`'s feed, signed by host `signer`'s machine key.
fn entry(c: &mut Choices<'_>, author: usize, signer: usize) -> FeedEntry {
    let kind = *c.pick(&Kind::ALL);
    let mut e = net::entry(author, kind, u64::from(c.u8() % 4) + 1, serde_json::json!({ "text": xenonet_fuzz::text(c.chunk()) }));
    if signer != author {
        let tag = Purpose::FEED.tag().len() + 1;
        e.sig = net::machine_key(signer).sign(Purpose::FEED, &e.signing_bytes()[tag..]).expect("a signature");
    }
    e
}

/// A sealed envelope of the fixed network, and whether laptop should open
/// it: honest, from a member, sealed for laptop and named for it.
fn built(c: &mut Choices<'_>) -> (Envelope, bool) {
    let author = c.below(net::HOSTS.len());
    let signer = if c.rarely(24) { c.below(net::HOSTS.len()) } else { author };
    let named = if c.rarely(32) { c.below(net::HOSTS.len()) } else { LAPTOP };
    let sealed_for = if c.rarely(32) { c.below(net::HOSTS.len()) } else { named };
    let e = entry(c, author, signer);
    let mut eph = [0x80u8; 32];
    eph[1] = c.u8();
    let env = Envelope::seal_entry(&e, &net::machine(named).id(), &net::public(sealed_for), Some(eph)).expect("sealing with fixed keys");
    let member = view().by_machine(&e.author).is_some();
    (env, signer == author && named == LAPTOP && sealed_for == LAPTOP && member)
}

fn changed(c: &mut Choices<'_>, by_field: bool) {
    let (env, honest) = built(c);
    let v = view();
    let bytes = if !c.bool() {
        env.encode()
    } else if by_field {
        let pick = c.u8();
        net::change_json_field(&env.to_json(), pick, c.value()).into_bytes()
    } else {
        net::change_bytes(&String::from_utf8_lossy(&env.encode()), c)
    };
    let Ok(g) = Envelope::decode(&bytes) else { return };
    round_trips(&g);
    let got = open_for_laptop(&g, &v);
    if g == env && honest {
        assert!(got.is_some(), "laptop didn't open what its author sealed for it: {env:?}");
    }
    if let Some(got) = got {
        assert!(honest && g == env, "laptop opened a changed or misdirected envelope:\nwas {env:?}\nnow {g:?}");
        took_rightly(&g, &got, &v);
    }
}

fn live(c: &mut Choices<'_>) {
    let author = c.below(net::HOSTS.len());
    let from = if c.bool() { author } else { c.below(net::HOSTS.len()) };
    let body = c.value();
    let body = if body.is_object() { body } else { serde_json::json!({ "op": "delta", "x": body }) };
    let Ok(env) = Envelope::live(net::NET, &net::machine(author).id(), "presence", net::now() as u64, &body) else { return };
    let bytes = match c.below(3) {
        0 => env.encode(),
        1 => {
            let pick = c.u8();
            net::change_json_field(&env.to_json(), pick, c.value()).into_bytes()
        }
        _ => net::change_bytes(&String::from_utf8_lossy(&env.encode()), c),
    };
    let Ok(g) = Envelope::decode(&bytes) else { return };
    round_trips(&g);
    let v = view();
    match g.open_live(&net::public(from).sign, &v) {
        Ok(b) => {
            assert_eq!(g.class, Class::Live);
            assert!(v.by_machine(&g.author).is_some_and(|m| m.keys == net::public(from)), "took a live envelope from another's connection: {g:?}");
            assert!(b.is_object());
        }
        Err(_) => assert!(!(g == env && from == author && v.by_machine(&env.author).is_some()), "refused its author's own live envelope: {env:?}"),
    }
    assert!(open_for_laptop(&g, &v).is_none(), "a live envelope opened as an entry");
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    match c.u8() % 4 {
        0 => whole(c.rest()),
        1 => changed(&mut c, true),
        2 => changed(&mut c, false),
        _ => live(&mut c),
    }
});
