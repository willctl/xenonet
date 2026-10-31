//! Pairing: one code joins a machine to a home machine as a satellite (docs/sync-design.md).
//!
//! On home, `xenolith pair offer` makes a code, `<home-id>.<secret>`, valid for ten minutes and one
//! join. On the satellite, `xenolith pair join --url <base>` makes a keypair, posts its public key
//! and an HMAC of the join under the secret to /sync/v1/pair, and home answers with a machine token
//! sealed to that key. Seeing a code is not enough to get a token: the proof needs the secret, and
//! the first valid join uses the code up. The token is `ctx:read` and `ctx:append` for every space
//! and nothing else (machine:local is never in "all").
//!
//! The sealing: X25519 between the satellite's key and a key home makes for this answer, mixed with
//! the secret into two keys by HMAC-SHA256; the token is XORed with an HMAC-SHA256 keystream and the
//! answer carries a MAC of the whole, so the satellite also knows the answer came from the home that
//! made the code.
//!
//! pair.json in the config folder holds this machine's home id and the offer waiting, for you alone.

use crate::access::{self, Scope};
use crate::paths::AppPaths;
use crate::sync::{self, Config};
use crate::text::{hex, random_hex};
use crate::wiki::{Result, wiki_err};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use x25519_dalek::{PublicKey, StaticSecret};

pub const ROUTE: &str = "/sync/v1/pair";
/// How long a code can be used.
pub const CODE_TTL_MS: i64 = 10 * 60 * 1000;
/// The scopes a paired machine's token has: read and leave notes, in every space but machine:local.
/// How long the grant behind a paired machine's token lasts: ten years, or until `xenolith revoke`.
pub const GRANT_MS: i64 = 3650 * 24 * 3600 * 1000;
pub const TOKEN_SCOPES: [&str; 2] = ["ctx:read:all", "ctx:append:all"];

const DOMAIN: &[u8] = b"xenolith-pair-v1";

pub fn state_file(paths: &AppPaths) -> PathBuf {
    paths.config_dir.join("pair.json")
}

// ---------------------------------------------------------------- crypto

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    for p in parts {
        inner.update(p);
    }
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn keystream(key: &[u8; 32], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut i = 0u64;
    while out.len() < len {
        out.extend_from_slice(&hmac(key, &[b"stream", &i.to_be_bytes()]));
        i += 1;
    }
    out.truncate(len);
    out
}

fn xor(data: &[u8], stream: &[u8]) -> Vec<u8> {
    data.iter().zip(stream).map(|(a, b)| a ^ b).collect()
}

/// The two keys of one exchange: encryption and MAC.
fn keys(secret: &str, shared: &[u8; 32], theirs: &[u8; 32], ours: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let prk = hmac(secret.as_bytes(), &[DOMAIN, b"\0keys\0", shared, theirs, ours]);
    (hmac(&prk, &[b"enc"]), hmac(&prk, &[b"mac"]))
}

fn proof(secret: &str, home: &str, name: &str, key: &[u8; 32]) -> [u8; 32] {
    hmac(secret.as_bytes(), &[DOMAIN, b"\0join\0", home.as_bytes(), b"\0", name.as_bytes(), b"\0", key])
}

fn secret_key() -> StaticSecret {
    let mut b = [0u8; 32];
    let _ = getrandom::fill(&mut b);
    StaticSecret::from(b)
}

fn key_of(h: &str) -> Option<[u8; 32]> {
    unhex(h)?.try_into().ok()
}

// ---------------------------------------------------------------- home's state

#[derive(Clone, Debug, Default)]
struct Used {
    at: i64,
    name: String,
    token: String,
    source: String,
}

#[derive(Clone, Debug)]
struct Offer {
    id: String,
    secret: String,
    expires_at: i64,
    used: Option<Used>,
}

#[derive(Clone, Debug, Default)]
struct State {
    home: Option<String>,
    offer: Option<Offer>,
}

fn is_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl State {
    fn load(paths: &AppPaths) -> State {
        let v: Value = fs::read_to_string(state_file(paths)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let home = v["home"].as_str().filter(|h| is_hex(h, 16)).map(String::from);
        let o = &v["offer"];
        let offer = (|| {
            let used = o["used"].as_object().map(|_| Used {
                at: o["used"]["at"].as_i64().unwrap_or(0),
                name: o["used"]["name"].as_str().unwrap_or("").into(),
                token: o["used"]["token"].as_str().unwrap_or("").into(),
                source: o["used"]["source"].as_str().unwrap_or("").into(),
            });
            Some(Offer { id: o["id"].as_str()?.into(), secret: o["secret"].as_str().filter(|s| is_hex(s, 32))?.into(), expires_at: o["expiresAt"].as_i64()?, used })
        })();
        State { home, offer }
    }

    fn save(&self, paths: &AppPaths) -> Result<()> {
        let mut v = json!({ "home": self.home });
        if let Some(o) = &self.offer {
            v["offer"] = json!({ "id": o.id, "secret": o.secret, "expiresAt": o.expires_at });
            if let Some(u) = &o.used {
                v["offer"]["used"] = json!({ "at": u.at, "name": u.name, "token": u.token, "source": u.source });
            }
        }
        crate::private::write(&state_file(paths), &format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default()), &crate::private::service_readers())?;
        Ok(())
    }
}

fn locked<T>(paths: &AppPaths, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
    fs::create_dir_all(&paths.config_dir)?;
    let mut guard = match crate::lock::acquire(&paths.config_dir, "pair", &crate::lock::LockOpts::default()) {
        Ok(g) => g,
        Err(crate::lock::AcquireError::Busy(b)) => return wiki_err(format!("Pairing is busy ({}). Try again shortly.", b.0)),
        Err(crate::lock::AcquireError::Io(e)) => return Err(e.into()),
    };
    let mut st = State::load(paths);
    let r = f(&mut st);
    if r.is_ok() {
        st.save(paths)?;
    }
    guard.release();
    r
}

// ---------------------------------------------------------------- home: offer and accept

/// Makes a code for one join in the next ten minutes, replacing any code waiting. Returns the code
/// and when it lapses. A machine that is a satellite has no home to offer.
pub fn offer(paths: &AppPaths, now: i64) -> Result<(String, i64)> {
    if let Some(h) = Config::load(paths).home {
        return wiki_err(format!("This machine is a satellite of {h}: pairing codes come from home."));
    }
    let (code, expires, id) = locked(paths, |st| {
        let home = st.home.get_or_insert_with(|| random_hex(16)).clone();
        let (secret, id) = (random_hex(32), random_hex(8));
        let expires = now + CODE_TTL_MS;
        st.offer = Some(Offer { id: id.clone(), secret: secret.clone(), expires_at: expires, used: None });
        Ok((format!("{home}.{secret}"), expires, id))
    })?;
    crate::audit::best_effort(paths, "pair", "you", vec![("event", json!("offer")), ("offer", json!(id)), ("expires", json!(expires))]);
    Ok((code, expires))
}

/// Why home turned a join away. `bad` joins count against the sender like a bad token.
#[derive(Debug)]
pub struct Refusal {
    pub status: u16,
    pub why: String,
    pub bad: bool,
}

fn refuse(status: u16, why: &str, bad: bool) -> Refusal {
    Refusal { status, why: why.into(), bad }
}

const NOT_VALID: &str = "That code is not valid, or it has run out. Ask home for a new one: xenolith pair offer.";

/// Home's answer to POST /sync/v1/pair: checks the proof, uses the code up, makes the machine's
/// token and seals it to the satellite's key. `source` is where the request came from, for the
/// audit chain.
pub fn accept(paths: &AppPaths, body: &Value, source: &str, now: i64) -> std::result::Result<Value, Refusal> {
    let (home, name) = (body["home"].as_str().unwrap_or(""), body["name"].as_str().unwrap_or(""));
    let (Some(theirs), Some(sent)) = (body["key"].as_str().and_then(key_of), body["proof"].as_str().and_then(unhex)) else {
        return Err(refuse(400, "Send {\"home\", \"name\", \"key\", \"proof\"} as JSON.", true));
    };
    if !sync::valid_name(name) || !is_hex(home, 16) {
        return Err(refuse(400, "A machine name is lowercase letters, digits, '-' and '.'.", true));
    }
    let result = locked(paths, |st| {
        let Some(offer) = st.offer.as_mut().filter(|_| st.home.as_deref() == Some(home)) else { return Ok(Err(refuse(403, NOT_VALID, true))) };
        // Without the secret the proof cannot be made: nothing below says more than NOT_VALID until it is.
        if !same(&proof(&offer.secret, home, name, &theirs), &sent) {
            return Ok(Err(refuse(403, NOT_VALID, true)));
        }
        if let Some(u) = &offer.used {
            let (id, who) = (offer.id.clone(), u.clone());
            crate::audit::best_effort(
                paths,
                "refused",
                "a paired machine",
                vec![
                    ("why", json!("pair code already used")),
                    ("offer", json!(id)),
                    ("first", json!(format!("machine:{} ({}) from {}", who.name, who.token, who.source))),
                    ("source", json!(source)),
                    ("transport", json!("sync")),
                ],
            );
            return Ok(Err(refuse(403, "That code was already used.", true)));
        }
        if now > offer.expires_at {
            return Ok(Err(refuse(403, NOT_VALID, true)));
        }
        // Used up before the token exists: a second join racing this one finds it taken.
        offer.used = Some(Used { at: now, name: name.into(), token: String::new(), source: source.into() });
        let secret = offer.secret.clone();
        let scopes: Vec<Scope> = TOKEN_SCOPES.iter().filter_map(|s| access::parse_scope(s)).collect();
        // A token reaches a space only with a grant for its subject too (the lower of the two wins):
        // append includes read, and the grant lasts until it is revoked in practice.
        let issued = access::update(paths, |p| {
            let subject = format!("machine:{name}");
            p.grant(&subject, access::parse_scope("ctx:append:all").unwrap_or_else(|| unreachable!()), GRANT_MS, "pair", now)?;
            p.issue(&subject, scopes, &format!("paired {name}"), None, None, now)
        });
        let (token, rec) = match issued {
            Ok(t) => t,
            Err(e) => {
                st.offer = None;
                return Ok(Err(refuse(500, &format!("Could not make the token: {e}"), false)));
            }
        };
        if let Some(u) = st.offer.as_mut().and_then(|o| o.used.as_mut()) {
            u.token = rec.id.clone();
        }
        // The answer: sealed to the satellite's key with a key made for this answer.
        let ours = secret_key();
        let our_pub = PublicKey::from(&ours).to_bytes();
        let shared = ours.diffie_hellman(&PublicKey::from(theirs));
        if !shared.was_contributory() {
            st.offer = None;
            let _ = access::update(paths, |p| Ok(p.revoke_token(&rec.id)));
            return Ok(Err(refuse(400, "That key is not usable.", true)));
        }
        let (enc, mac_key) = keys(&secret, shared.as_bytes(), &theirs, &our_pub);
        let sealed = xor(token.as_bytes(), &keystream(&enc, token.len()));
        let mac = hmac(&mac_key, &[DOMAIN, b"\0answer\0", home.as_bytes(), &theirs, &our_pub, &sealed]);
        Ok(Ok((json!({ "v": 1, "home": home, "key": hex(&our_pub), "sealed": hex(&sealed), "mac": hex(&mac), "tokenId": rec.id }), rec.id)))
    });
    match result {
        Err(e) => Err(refuse(500, &e.to_string(), false)),
        Ok(Err(r)) => Err(r),
        Ok(Ok((answer, token_id))) => {
            crate::audit::best_effort(
                paths,
                "pair",
                "a paired machine",
                vec![
                    ("event", json!("joined")),
                    ("subject", json!(format!("machine:{name}"))),
                    ("token", json!(token_id)),
                    ("source", json!(source)),
                    ("scopes", json!(TOKEN_SCOPES)),
                    ("grant", json!("ctx:append:all")),
                ],
            );
            Ok(answer)
        }
    }
}

// ---------------------------------------------------------------- satellite: join

/// Splits `<home-id>.<secret>`.
pub fn parse_code(code: &str) -> Option<(&str, &str)> {
    let (home, secret) = code.trim().split_once('.')?;
    (is_hex(home, 16) && is_hex(secret, 32)).then_some((home, secret))
}

/// What a join set up.
pub struct Joined {
    pub peer: String,
    pub machine: String,
    pub token_id: String,
    pub spaces: usize,
}

/// Pairs this machine with the home at `url`: the token comes back sealed to a key made here, and
/// is kept as peers/<peer>.token. This machine then follows every space home lists.
pub fn join(paths: &AppPaths, root: &std::path::Path, url: &str, code: &str, machine: &str, peer: &str) -> std::result::Result<Joined, String> {
    let url = url.trim().trim_end_matches('/');
    if !sync::valid_url(url) {
        return Err("the URL must be https://, or http:// to 127.0.0.1 or localhost (a code and a token never cross a network in clear)".into());
    }
    if !sync::valid_name(machine) || !sync::valid_name(peer) {
        return Err("a machine or peer name is lowercase letters, digits, '-' and '.'".into());
    }
    let (home, secret) = parse_code(code).ok_or("that is not a pairing code: it looks like <16 characters>.<32 characters>")?;
    let mine = secret_key();
    let my_pub = PublicKey::from(&mine).to_bytes();
    let body = json!({ "v": 1, "home": home, "name": machine, "key": hex(&my_pub), "proof": hex(&proof(secret, home, machine, &my_pub)) });
    let (status, answer) = sync::post_open(&format!("{url}{ROUTE}"), &body, Config::load(paths).timeout()).map_err(|e| format!("home could not be reached: {e}"))?;
    if status != 200 {
        let why = answer["error_description"].as_str().or(answer["error"].as_str()).unwrap_or("no reason given");
        return Err(format!("home answered {status}: {why}"));
    }
    let sealed = answer["sealed"].as_str().and_then(unhex).ok_or("home's answer is not what a pairing answer looks like")?;
    let (their_pub, mac) = (answer["key"].as_str().and_then(key_of), answer["mac"].as_str().and_then(unhex));
    let (Some(their_pub), Some(mac)) = (their_pub, mac) else { return Err("home's answer is not what a pairing answer looks like".into()) };
    let shared = mine.diffie_hellman(&PublicKey::from(their_pub));
    if !shared.was_contributory() {
        return Err("home's key is not usable".into());
    }
    let (enc, mac_key) = keys(secret, shared.as_bytes(), &my_pub, &their_pub);
    if !same(&hmac(&mac_key, &[DOMAIN, b"\0answer\0", home.as_bytes(), &my_pub, &their_pub, &sealed]), &mac) {
        return Err("home's answer did not check out: it was not made with this code. Nothing was changed.".into());
    }
    let token = String::from_utf8(xor(&sealed, &keystream(&enc, sealed.len()))).map_err(|_| "home's token is not text")?;
    if !access::is_token(&token) {
        return Err("home's answer holds no token".into());
    }
    let token_id = answer["tokenId"].as_str().map(String::from).unwrap_or_else(|| format!("tok_{}", &access::hash_token(&token)[..12]));
    sync::save_token(paths, peer, &token).map_err(|e| e.to_string())?;
    sync::update(paths, |c| {
        c.peers.insert(peer.to_string(), url.to_string());
        c.machine = Some(machine.to_string());
        c.home = Some(peer.to_string());
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    crate::audit::best_effort(paths, "pair", "you", vec![("event", json!("paired")), ("home", json!(peer)), ("as", json!(format!("machine:{machine}"))), ("token", json!(token_id))]);
    let spaces = sync::adopt_home_spaces(paths, root).map_err(|e| format!("paired, but home's spaces could not be listed yet: {e}"))?;
    Ok(Joined { peer: peer.into(), machine: machine.into(), token_id, spaces })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_the_rfc_4231_vectors() {
        assert_eq!(hex(&hmac(&[0x0b; 20], &[b"Hi There"])), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        assert_eq!(hex(&hmac(b"Jefe", &[b"what do ya want ", b"for nothing?"])), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        let long = [0xaa; 131];
        assert_eq!(hex(&hmac(&long, &[b"Test Using Larger Than Block-Size Key - Hash Key First"])), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn a_code_has_a_home_and_a_secret() {
        assert!(parse_code("0123456789abcdef.0123456789abcdef0123456789abcdef").is_some());
        assert!(parse_code("0123456789abcdef").is_none() && parse_code("x.y").is_none() && parse_code("").is_none());
    }

    #[test]
    fn the_keystream_round_trips() {
        let key = [7u8; 32];
        let data = b"xenolith_0123".to_vec();
        assert_eq!(xor(&xor(&data, &keystream(&key, data.len())), &keystream(&key, data.len())), data);
        assert_ne!(xor(&data, &keystream(&key, data.len())), data);
    }
}
