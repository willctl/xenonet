// Window reads' rules, on their own: the cert, the read's signature, its
// time and nonce, and what a window may read. svc/2's streams themselves
// run on iroh in tests/svc_replay.rs and the service's net_svc.rs.

use super::*;

const NET: &str = "net_00112233445566778899aabbccddeeff";
const T: i64 = 1_792_512_000_000;

struct Person {
    device: SeedSigner,
}

impl Person {
    fn new() -> Person {
        Person { device: SeedSigner::from_seed(&[3; 32], KeyHome::File).unwrap() }
    }

    fn device_id(&self) -> String {
        self.device.public().id()
    }

    // A window key for `endpoint`, certified by this person's device at `at`.
    fn key(&self, endpoint: PeerId, at: i64) -> WindowKey {
        WindowKey::make(endpoint, "prs_will", &self.device_id(), at, |msg| Ok(self.device.sign(CERT, msg)?)).unwrap()
    }
}

const LAPTOP: PeerId = PeerId([1; 32]);
const STUDIO: PeerId = PeerId([2; 32]);
const DESK: PeerId = PeerId([3; 32]);

// What studio checks a read from `peer` against.
fn at_studio<'a>(p: &Person, device: &'a str, peer: PeerId, now: i64) -> WindowCheck<'a> {
    WindowCheck { me: STUDIO, peer, owner: Some("prs_will"), device_there: Some(device), device_key: Some(p.device.public()), now }
}

fn code(r: Result<(), Answer>) -> Option<(u16, String)> {
    r.err().map(|a| (a.status, a.code.unwrap_or_default()))
}

#[test]
fn a_read_from_the_window_its_cert_names_is_taken_once() {
    let p = Person::new();
    let device = p.device_id();
    let key = p.key(LAPTOP, T);
    let mut nonces = Nonces::default();
    let read = key.read(NET, STUDIO, "/api/session/claude:abc/transcript?limit=120", T + 1000).unwrap();
    // It crosses as JSON and reads back the same.
    let back = WindowRead::parse(&read.head()).unwrap();
    assert_eq!(back, read);
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T + 2000).check(&back, &mut nonces)), None);
    // Sent again, on any connection: refused.
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T + 3000).check(&back, &mut nonces)), Some((409, "replayed".into())));
    // A new read under the same cert goes, with no new signature from the
    // device.
    let next = key.read(NET, STUDIO, "/api/session/claude:abc/stats", T + 4000).unwrap();
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T + 4000).check(&next, &mut nonces)), None);
}

#[test]
fn a_cert_is_no_good_from_another_machines_connection() {
    let p = Person::new();
    let device = p.device_id();
    let key = p.key(LAPTOP, T);
    let read = key.read(NET, STUDIO, "/api/session/claude:abc/transcript", T).unwrap();
    // desk holds laptop's cert and a read signed by its key, and sends it
    // over its own connection.
    assert_eq!(code(at_studio(&p, &device, DESK, T).check(&read, &mut Nonces::default())), Some((403, "cert-elsewhere".into())));
    // A read made for studio, sent to desk.
    let desk = WindowCheck { me: DESK, ..at_studio(&p, &device, LAPTOP, T) };
    assert_eq!(code(desk.check(&read, &mut Nonces::default())), Some((403, "bad-signature".into())));
    // A read for another network than it names.
    let mut other = read.clone();
    other.network = "net_ffeeddccbbaa99887766554433221100".into();
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T).check(&other, &mut Nonces::default())), Some((403, "bad-signature".into())));
    // A path changed on the way.
    let mut path = read.clone();
    path.path = "/api/session/claude:other/transcript".into();
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T).check(&path, &mut Nonces::default())), Some((403, "bad-signature".into())));
}

#[test]
fn only_the_owners_device_on_that_machine_certifies_and_only_for_twelve_hours() {
    let p = Person::new();
    let device = p.device_id();
    let key = p.key(LAPTOP, T);
    let read = |now: i64| key.read(NET, STUDIO, "/api/session/claude:abc/stats", now).unwrap();
    let mut n = Nonces::default();
    // Another person's network.
    let c = WindowCheck { owner: Some("prs_other"), ..at_studio(&p, &device, LAPTOP, T) };
    assert_eq!(code(c.check(&read(T), &mut n)), Some((403, "not-the-person".into())));
    // The chain names another device on laptop.
    assert_eq!(code(at_studio(&p, "xns1_another", LAPTOP, T).check(&read(T), &mut n)), Some((403, "not-its-device".into())));
    // The device was revoked: the person's chain has it no more.
    let c = WindowCheck { device_key: None, ..at_studio(&p, &device, LAPTOP, T) };
    assert_eq!(code(c.check(&read(T), &mut n)), Some((403, "not-its-device".into())));
    // Signed by some other key than the device's.
    let forged = WindowKey::make(LAPTOP, "prs_will", &device, T, |msg| Ok(SeedSigner::from_seed(&[9; 32], KeyHome::File).unwrap().sign(CERT, msg)?)).unwrap();
    let r = forged.read(NET, STUDIO, "/api/session/claude:abc/stats", T).unwrap();
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T).check(&r, &mut n)), Some((403, "bad-cert".into())));
    // Past its 12 hours.
    let late = T + CERT_MS + 1;
    assert_eq!(code(at_studio(&p, &device, LAPTOP, late).check(&read(late), &mut n)), Some((403, "cert-expired".into())));
    assert!(key.fresh(T + CERT_MS - CERT_RENEW_MS - 1) && !key.fresh(T + CERT_MS - CERT_RENEW_MS));
    // A cert that claims more than 12 hours, signed as such.
    let long = WindowCert::signed("prs_will", &device, &key.cert.key, LAPTOP, T, T + 2 * CERT_MS);
    let mut stretched = read(T);
    stretched.cert.until = T + 2 * CERT_MS;
    stretched.cert.sig = p.device.sign(CERT, &long).unwrap();
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T).check(&stretched, &mut n)), Some((403, "cert-expired".into())));
}

#[test]
fn a_read_is_taken_within_two_minutes_of_its_time() {
    let p = Person::new();
    let device = p.device_id();
    let key = p.key(LAPTOP, T);
    let mut n = Nonces::default();
    let read = key.read(NET, STUDIO, "/api/session/claude:abc/stats", T + 60_000).unwrap();
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T + 60_000 + READ_SKEW_MS + 1).check(&read, &mut n)), Some((403, "clock-skew".into())));
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T + 60_000 - READ_SKEW_MS - 1).check(&read, &mut n)), Some((403, "clock-skew".into())));
    assert_eq!(code(at_studio(&p, &device, LAPTOP, T + 60_000 + READ_SKEW_MS).check(&read, &mut n)), None);
}

#[test]
fn a_window_reads_transcripts_and_stats_and_titles_only_keeps_transcripts() {
    let ok = |p: &str, t: bool| window_path(p, t).map_err(|a| (a.status, a.code.unwrap_or_default()));
    assert_eq!(ok("/api/session/claude:abc/transcript?limit=120&cursor=c:1", false), Ok("/api/session/claude:abc/transcript?limit=120&cursor=c:1".into()));
    assert_eq!(ok("/api/session/codex:abc/stats", false), Ok("/api/session/codex:abc/stats".into()));
    assert_eq!(ok("/api/session/claude:abc/transcript", true), Err((403, "titles-only".into())));
    assert_eq!(ok("/api/session/claude:abc/stats", true), Ok("/api/session/claude:abc/stats".into()), "stats are served with titles only");
    for (bad, status) in [
        ("/api/graph", 403),
        ("/api/collision?a=claude:a&b=claude:b", 403),
        ("/api/session/claude:abc/messages", 403),
        ("/api/session/claude:../transcript", 400),
        ("/api/session/@laptop/claude:abc/transcript", 400),
        ("/api/session/claude:abc/transcript?limit=99999", 400),
        ("/api/session/claude:abc/transcript?detail=1", 400),
        ("/api/session/claude:abc/stats?x=1", 403),
    ] {
        assert_eq!(ok(bad, false).map_err(|e| e.0), Err(status), "{bad}");
    }
}

#[test]
fn heads_that_arent_reads_are_refused() {
    let p = Person::new();
    let key = p.key(LAPTOP, T);
    let head = key.read(NET, STUDIO, "/api/session/claude:abc/stats", T).unwrap().head();
    for (k, v) in [("op", json!("request")), ("network", json!("net_x")), ("nonce", json!("short")), ("cert", json!({})), ("sig", json!("+")), ("at", json!("now"))] {
        let mut bad = head.clone();
        bad[k] = v;
        assert!(WindowRead::parse(&bad).is_err(), "{k}");
    }
    assert_eq!(WindowCert::parse(&key.cert.to_json()), Some(key.cert.clone()));
}
