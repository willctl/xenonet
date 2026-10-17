//! core/redact.js: what an export of the day may say. Projects show as star
//! names made from a keyed hash of each one's id: the same all day, new the
//! next day. The key is a salt kept in XenoNet's state folder.

use crate::export_labels::is_labelled;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;
use xenonet_base::json::number;
use xenonet_base::jsstr;
use xenonet_base::text::{js_string, truthy};

const ONSETS: [&str; 20] = ["b", "d", "f", "g", "h", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "th", "sh", "kr", "dr", "vr"];
const VOWELS: [&str; 10] = ["a", "e", "i", "o", "u", "ae", "ai", "ei", "io", "ou"];
const CODAS: [&str; 8] = ["", "", "n", "r", "s", "l", "th", "x"];

/// The day a name holds for, as the local date.
pub fn day_key(now: f64) -> String {
    jsstr::local_day(now)
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(msg).finalize();
    Sha256::new().chain_update(opad).chain_update(inner).finalize().into()
}

fn digest(salt: &str, day: &str, id: &str) -> [u8; 32] {
    hmac_sha256(salt.as_bytes(), format!("{day}\u{0}{id}").as_bytes())
}

// A name spelt from the bytes of a hash, `syllables` long.
fn spell(bytes: &[u8; 32], syllables: usize) -> String {
    let mut s = String::new();
    for i in 0..syllables {
        let (a, b, c) = (bytes[i * 3] as usize, bytes[i * 3 + 1] as usize, bytes[i * 3 + 2] as usize);
        s.push_str(ONSETS[a % ONSETS.len()]);
        s.push_str(VOWELS[b % VOWELS.len()]);
        if i == syllables - 1 {
            s.push_str(CODAS[c % CODAS.len()]);
        }
    }
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + chars.as_str())
}

/// One star name, two or three syllables, from the hash of the id under
/// the salt and the day.
pub fn star_name(id: &str, salt: &str, day: &str) -> String {
    let bytes = digest(salt, day, id);
    spell(&bytes, 2 + (bytes[31] % 2) as usize)
}

/// Star names for a set of ids, each one different: a name two ids share
/// grows a syllable for the one whose hash sorts later.
pub fn star_names(ids: &[Value], salt: &str, day: &str) -> Vec<(Value, String)> {
    let mut unique: Vec<&Value> = Vec::new();
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    let mut hashed: Vec<(&Value, [u8; 32])> = unique.into_iter().map(|id| (id, digest(salt, day, &js_string(id)))).collect();
    hashed.sort_by(|a, b| a.1.cmp(&b.1));
    let mut taken = std::collections::HashSet::new();
    let mut names = Vec::new();
    for (id, bytes) in hashed {
        let mut n = 2 + (bytes[31] % 2) as usize;
        let mut name = spell(&bytes, n);
        let mut k = 2;
        while taken.contains(&name) {
            name = if n < 5 {
                n += 1;
                spell(&bytes, n)
            } else {
                format!("{} {k}", spell(&bytes, 5))
            };
            k += 1;
        }
        taken.insert(name.clone());
        names.push((id.clone(), name));
    }
    names
}

static RUN_SALT: Mutex<Option<String>> = Mutex::new(None);

fn new_salt() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the system's randomness");
    jsstr::hex(&bytes)
}

/// The salt, kept in the state folder as poster-salt: made the first time
/// an export needs it, readable by you alone. Without a folder, a salt for
/// this run alone.
pub fn poster_salt(dir: Option<&str>) -> std::io::Result<String> {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else {
        return Ok(RUN_SALT.lock().unwrap().get_or_insert_with(new_salt).clone());
    };
    let file = Path::new(dir).join("poster-salt");
    if let Ok(b) = std::fs::read(&file) {
        let kept = String::from_utf8_lossy(&b);
        let kept = xenonet_base::text::js_trim(&kept);
        if kept.len() == 64 && kept.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
            return Ok(kept.to_string());
        }
    }
    let salt = new_salt();
    std::fs::create_dir_all(dir)?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    std::io::Write::write_all(&mut o.open(&file)?, format!("{salt}\n").as_bytes())?;
    Ok(salt)
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(f64::NAN)
}

fn or0(v: Option<&Value>) -> Value {
    v.filter(|v| !v.is_null()).cloned().unwrap_or(Value::from(0))
}

/// The day's archetype: a lone star, a binary, a pulsar (the work came in
/// a burst), a swarm or a cluster; a quiet sky without sessions.
pub fn archetype_of(day: Option<&Value>) -> &'static str {
    let totals = day.and_then(|d| d.get("totals"));
    let n = totals.and_then(|t| t.get("sessions")).filter(|v| !v.is_null()).map_or(0.0, |v| num(Some(v)));
    if n == 0.0 {
        return "quiet sky";
    }
    if n == 1.0 {
        return "lone star";
    }
    if n == 2.0 {
        return "binary";
    }
    let hours: Vec<f64> = day.and_then(|d| d.get("hours")).and_then(Value::as_array).map_or_else(Vec::new, |h| h.iter().map(|x| num(Some(x))).collect());
    let calls = hours.iter().fold(0.0, |a, b| a + b);
    let peak = hours.iter().fold(0.0f64, |a, &b| if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) });
    if calls >= 20.0 && peak >= calls * 0.4 {
        return "pulsar";
    }
    if n >= 6.0 {
        "swarm"
    } else {
        "cluster"
    }
}

// The ids a day's names cover: the brightest projects and every labelled
// node on the map.
fn named_ids(day: Option<&Value>, nodes: &[Value]) -> Vec<Value> {
    let projects = day.and_then(|d| d.get("topProjects")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    projects
        .iter()
        .map(|p| p.get("id").cloned().unwrap_or(Value::Null))
        .chain(nodes.iter().filter(|n| is_labelled(n.get("kind"))).map(|n| n.get("id").cloned().unwrap_or(Value::Null)))
        .collect()
}

fn lookup<'a>(names: &'a [(Value, String)], id: Option<&Value>) -> Option<&'a str> {
    let id = id.cloned().unwrap_or(Value::Null);
    names.iter().find(|(k, _)| *k == id).map(|(_, n)| n.as_str())
}

/// What a poster or film gets of the day's log: the date, the archetype,
/// the totals, the tokens, the brightest stars by star name and how many
/// sessions ran in each app; with `names`, each star's project too.
pub fn constellation_of(day: Option<&Value>, salt: &str, now: f64, names: bool, nodes: &[Value]) -> Value {
    let day = day.filter(|d| !d.is_null());
    let field = |k: &str| day.and_then(|d| d.get(k)).filter(|v| !v.is_null());
    let projects = field("topProjects").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let stars = star_names(&named_ids(day, nodes), salt, &day_key(now));
    let mut apps: Map<String, Value> = Map::new();
    for s in field("sessions").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
        let app = s.get("app").filter(|a| truthy(Some(a))).map_or_else(|| "other".to_string(), js_string);
        let n = apps.get(&app).and_then(Value::as_f64).unwrap_or(0.0) + 1.0;
        apps.insert(app, number(n));
    }
    let t = field("totals");
    let total = |k: &str| or0(t.and_then(|t| t.get(k)));
    let stars: Vec<Value> = projects
        .iter()
        .map(|p| {
            let mut m = Map::new();
            m.insert("name".into(), lookup(&stars, p.get("id")).map_or(Value::Null, |n| Value::String(n.into())));
            m.insert("toolCalls".into(), or0(p.get("toolCalls")));
            if names {
                m.insert("project".into(), p.get("name").cloned().unwrap_or(Value::Null));
            }
            Value::Object(m)
        })
        .collect();
    json!({
        "date": field("date").cloned().unwrap_or(Value::String(String::new())),
        "archetype": archetype_of(day),
        "totals": { "sessions": total("sessions"), "turns": total("turns"), "toolCalls": total("toolCalls"), "files": total("files") },
        "tokens": or0(field("tokens")),
        "stars": stars,
        "apps": Value::Object(xenonet_base::jsstr::js_key_order(apps)),
    })
}

/// The star name of each labelled node on the map, by id, from the same
/// names the day's constellation gets.
pub fn labels_of(nodes: &[Value], day: Option<&Value>, salt: &str, now: f64) -> Value {
    let stars = star_names(&named_ids(day.filter(|d| !d.is_null()), nodes), salt, &day_key(now));
    let mut labels = Map::new();
    for n in nodes.iter().filter(|n| is_labelled(n.get("kind"))) {
        let key = n.get("id").map_or_else(|| "undefined".into(), js_string);
        labels.insert(key, lookup(&stars, n.get("id")).map_or(Value::Null, |s| Value::String(s.into())));
    }
    Value::Object(xenonet_base::jsstr::js_key_order(labels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_is_rfc_4231s() {
        // RFC 4231, test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn names_differ_and_hold_all_day() {
        let salt = "a".repeat(64);
        let a = star_name("project:/x", &salt, "2026-10-03");
        assert_eq!(star_name("project:/x", &salt, "2026-10-03"), a);
        assert_ne!(star_name("project:/x", &salt, "2026-10-04"), a);
        let ids: Vec<Value> = (0..2000).map(|i| Value::String(format!("project:/r-{i}"))).collect();
        let names = star_names(&ids, &salt, "2026-10-03");
        let set: std::collections::HashSet<_> = names.iter().map(|(_, n)| n.clone()).collect();
        assert_eq!(set.len(), 2000);
        assert_eq!(archetype_of(Some(&json!({ "totals": { "sessions": 3 }, "hours": [30, 5] }))), "pulsar");
        assert_eq!(archetype_of(None), "quiet sky");
    }
}
