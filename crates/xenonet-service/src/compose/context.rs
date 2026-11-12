//! The shared context at the window (phase 6, revisions 7, 21 and 23):
//! what Xenolith on this machine says of the spaces it curates, over its
//! forward session (`say`), what the window offers for each, and signing
//! one as the person.
//!
//! - **Spaces.** Xenolith says which spaces it curates, what each is about,
//!   who owns it and the networks it's shared into. That's its word, for
//!   showing: what the person signs is decided again from the draft itself.
//! - **Offers.** A space the standalone owner key owns: make it yours (a
//!   move to the person, with presence). One of yours: share it into each
//!   network the grant reaches that it isn't in yet, "from the project"
//!   when it's a project's space and the project is shared into that
//!   network (revision 7); and stop sharing it into one, marked when the
//!   project's sharing narrowed there.
//! - **Signing.** The window asks Xenolith to draft the entry (`draft`),
//!   checks the draft does exactly what was asked (xenonet-net's
//!   `space_sign`), has the person's device sign it, with their presence
//!   where it widens, and hands it back to be sequenced (`signed`).
//! - **Narrowing.** Xenolith is told what each network sees of this
//!   machine's projects (`sharing`) as it attaches and whenever that
//!   changes, and holds a project's space back from a network the project
//!   isn't shared into any more at once; the Unshare that records it in the
//!   chain needs no presence, and is offered here.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;
use tokio::sync::oneshot;
use xenonet_net::forward::Forwards;
use xenonet_net::nodes::Nodes;
use xenonet_net::space_sign::Asked;
use xenonet_net::space_sign::Level;
use xenonet_os::sync::{Locked, ReadWrite};

/// The ALPN Xenolith's session is on.
pub const ALPN: &str = "xenolith/sync/2";
/// How long the window waits on Xenolith's answer to a draft or a sequence.
const ANSWER: Duration = Duration::from_secs(15);
/// The most spaces kept of what Xenolith says.
const MAX_SPACES: usize = 512;

/// The window's side of Xenolith's session.
pub struct Context {
    forwards: Weak<Forwards>,
    nodes: RwLock<Weak<Nodes>>,
    spaces: Mutex<Vec<Value>>,
    /// What Xenolith was last told of the networks' sharing.
    told: Mutex<Value>,
    pending: Mutex<BTreeMap<u64, oneshot::Sender<Value>>>,
    next: AtomicU64,
    /// When the window last asked Xenolith to say its spaces again.
    listed: Mutex<Option<std::time::Instant>>,
}

fn word(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 256).map(str::to_string)
}

// A space as Xenolith says it, kept only in this shape.
fn space_of(v: &Value) -> Option<Value> {
    let id = word(v, "id").filter(|i| i.starts_with("spc_"))?;
    let networks: Vec<Value> = v
        .get("networks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|n| Some(json!({ "network": word(n, "network").filter(|x| x.starts_with("net_"))?, "level": Level::parse(n.get("level")?.as_str()?)?.as_str() })))
        .take(64)
        .collect();
    Some(json!({
        "id": id,
        "address": word(v, "address").unwrap_or_default(),
        "about": word(v, "about").unwrap_or_default(),
        "owner": word(v, "owner").unwrap_or_default(),
        "weaker": v.get("weaker") == Some(&Value::Bool(true)),
        "networks": networks,
    }))
}

/// The ask the window's body names: `op` moveOwner, shareNetwork (with
/// `network` and `level`, append by default) or unshareNetwork.
pub fn asked_of(body: &Value) -> Result<Asked, String> {
    let network = || word(body, "network").filter(|n| n.starts_with("net_")).ok_or_else(|| "name the network (net_…)".to_string());
    match body.get("op").and_then(Value::as_str) {
        Some("moveOwner") => Ok(Asked::MoveOwner),
        Some("shareNetwork") => {
            let level = match body.get("level").and_then(Value::as_str) {
                None => Level::Append,
                Some(l) => Level::parse(l).ok_or_else(|| format!("{l} isn't a level (read, append, write, admin)"))?,
            };
            Ok(Asked::Share { network: network()?, level })
        }
        Some("unshareNetwork") => Ok(Asked::Unshare { network: network()? }),
        _ => Err("say what to sign: op moveOwner, shareNetwork or unshareNetwork".into()),
    }
}

impl Context {
    /// The window's side of what Xenolith says on `forwards`.
    pub fn new(forwards: &Arc<Forwards>) -> Arc<Context> {
        let c = Arc::new(Context {
            forwards: Arc::downgrade(forwards),
            nodes: RwLock::new(Weak::new()),
            spaces: Mutex::default(),
            told: Mutex::new(Value::Null),
            pending: Mutex::default(),
            next: AtomicU64::new(1),
            listed: Mutex::new(None),
        });
        let weak = Arc::downgrade(&c);
        forwards.set_said(Arc::new(move |alpn: &str, _actor: &str, body: &Value| {
            if let (ALPN, Some(c)) = (alpn, weak.upgrade()) {
                c.said(body);
            }
        }));
        c
    }

    /// The network's nodes: the person's identity, and what each network
    /// sees of this machine's projects.
    pub fn set_nodes(&self, nodes: &Arc<Nodes>) {
        *self.nodes.writing() = Arc::downgrade(nodes);
    }

    fn said(&self, body: &Value) {
        match body.get("t").and_then(Value::as_str) {
            Some("spaces") => {
                let list: Vec<Value> = body.get("spaces").and_then(Value::as_array).into_iter().flatten().filter_map(space_of).take(MAX_SPACES).collect();
                *self.spaces.locked() = list;
                // Its first word on a session: it's told the sharing anew.
                if body.get("hello") == Some(&Value::Bool(true)) {
                    *self.told.locked() = Value::Null;
                }
                self.changed();
            }
            Some("drafted" | "sequenced" | "failed") => {
                let req = body.get("req").and_then(Value::as_u64).unwrap_or(0);
                if let Some(tx) = self.pending.locked().remove(&req) {
                    let _ = tx.send(body.clone());
                }
            }
            _ => {}
        }
    }

    /// What each network this machine is in sees of its projects: "all",
    /// or the list of their ids.
    pub fn sharing(&self) -> Value {
        let Some(nodes) = self.nodes.reading().upgrade() else { return json!({}) };
        let mut out = serde_json::Map::new();
        for s in nodes.common().shareds() {
            if let Some(n) = s.network() {
                out.insert(n.id.clone(), s.sharing(&n.id).to_json());
            }
        }
        Value::Object(out)
    }

    /// The sharing may have changed: Xenolith is told when it did.
    pub fn changed(&self) {
        let now = self.sharing();
        let Some(f) = self.forwards.upgrade() else { return };
        let mut told = self.told.locked();
        if *told != now && f.tell(ALPN, json!({ "t": "sharing", "networks": now })) {
            *told = now;
        }
    }

    // Asks Xenolith `body` and waits for its answer.
    async fn ask(&self, mut body: Value) -> Result<Value, String> {
        let f = self.forwards.upgrade().ok_or("XenoNet's forwards aren't running")?;
        let req = self.next.fetch_add(1, Ordering::Relaxed);
        body["req"] = json!(req);
        let (tx, rx) = oneshot::channel();
        self.pending.locked().insert(req, tx);
        if !f.tell(ALPN, body) {
            self.pending.locked().remove(&req);
            return Err("Xenolith isn't attached to this XenoNet now".into());
        }
        let answer = tokio::time::timeout(ANSWER, rx).await;
        self.pending.locked().remove(&req);
        match answer {
            Ok(Ok(a)) if a["t"] == "failed" => Err(a["error"].as_str().unwrap_or("Xenolith refused").to_string()),
            Ok(Ok(a)) => Ok(a),
            _ => Err("Xenolith didn't answer".into()),
        }
    }

    /// The spaces Xenolith curates, each with what the window offers for
    /// it, while it's attached: none otherwise.
    pub fn view(&self) -> Value {
        let Some(f) = self.forwards.upgrade() else { return json!([]) };
        let Some(grant) = f.attached_grant(ALPN) else { return json!([]) };
        // Read, the list is asked for again (once a second at most), so the
        // next read has a space Xenolith's service founded since.
        {
            let mut at = self.listed.locked();
            if at.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
                *at = Some(std::time::Instant::now());
                f.tell(ALPN, json!({ "t": "list" }));
            }
        }
        let Some(nodes) = self.nodes.reading().upgrade() else { return json!([]) };
        let person = nodes.common().identity.person_id();
        let sharing = self.sharing();
        let mine: Vec<String> = sharing.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
        let reach: Vec<String> = mine.iter().filter(|n| grant.networks.as_ref().is_none_or(|g| g.contains(*n))).cloned().collect();
        let shows = |net: &str, project: &str| match &sharing[net] {
            Value::String(s) => s == "all",
            Value::Array(a) => a.iter().any(|p| p == project),
            _ => false,
        };
        let spaces = self.spaces.locked().clone();
        Value::Array(
            spaces
                .into_iter()
                .map(|mut s| {
                    let project = s["about"].as_str().filter(|a| a.starts_with("prj_")).map(str::to_string);
                    let shared: Vec<String> = s["networks"].as_array().into_iter().flatten().filter_map(|n| n["network"].as_str().map(str::to_string)).collect();
                    let mut offers = Vec::new();
                    if s["weaker"] == true {
                        offers.push(json!({ "op": "moveOwner" }));
                    } else if person.as_deref().is_some_and(|p| s["owner"] == format!("person:{p}").as_str()) {
                        let mut shares: Vec<Value> = reach
                            .iter()
                            .filter(|n| !shared.contains(n))
                            .map(|n| json!({ "op": "shareNetwork", "network": n, "level": "append", "fromProject": project.as_deref().is_some_and(|p| shows(n, p)) }))
                            .collect();
                        shares.sort_by_key(|o| o["fromProject"] != true);
                        offers.extend(shares);
                        for n in shared.iter().filter(|n| mine.contains(n)) {
                            let narrowed = project.as_deref().is_some_and(|p| !shows(n, p));
                            offers.push(json!({ "op": "unshareNetwork", "network": n, "narrowed": narrowed }));
                        }
                    }
                    s["offers"] = Value::Array(offers);
                    s
                })
                .collect(),
        )
    }

    /// Signs `asked` of `space` as the person: Xenolith drafts it, the
    /// draft is checked to do exactly that, the person's device signs it
    /// (with their presence where it widens), and Xenolith sequences it.
    /// The space as Xenolith then describes it. From the window alone.
    pub async fn sign(&self, space: &str, asked: &Asked, at_window: bool) -> Result<Value, String> {
        if !at_window {
            return Err("signing a space's entry as you is done from XenoNet's window".into());
        }
        let nodes = self.nodes.reading().upgrade().ok_or("the network isn't running")?;
        let identity = nodes.common().identity.clone();
        let person = identity.person_id().ok_or("this machine has no identity of yours yet: make or join a network first")?;
        let mut req = json!({ "t": "draft", "space": space, "personHead": identity.head() });
        match asked {
            Asked::MoveOwner => req["moveOwner"] = json!(person),
            Asked::Share { network, level } => {
                req["shareNetwork"] = json!(network);
                req["level"] = json!(level.as_str());
            }
            Asked::Unshare { network } => req["unshareNetwork"] = json!(network),
        }
        let draft = self.ask(req).await?;
        let now = xenonet_text::time::now_ms() as i64;
        let signed = xenonet_net::space_sign::sign(&identity, at_window, space, asked, &draft, now).map_err(|e| e.message)?;
        let field = if matches!(asked, Asked::Unshare { .. }) { "side" } else { "entry" };
        let done = self.ask(json!({ "t": "signed", "space": space, field: signed })).await?;
        Ok(done.get("space").cloned().unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_xenolith_says_of_a_space_is_kept_in_its_shape_alone() {
        let s = space_of(&json!({ "id": "spc_a", "address": "project:atlas", "about": "prj_x", "owner": "person:prs_p", "networks": [{ "network": "net_1", "level": "append" }, { "network": "evil", "level": "append" }, { "network": "net_2", "level": "god" }], "extra": 1 })).unwrap();
        assert_eq!(s["networks"], json!([{ "network": "net_1", "level": "append" }]));
        assert!(s.get("extra").is_none());
        assert!(space_of(&json!({ "id": "not-a-space" })).is_none());
    }

    #[test]
    fn the_windows_ask_is_one_of_three_and_names_its_network() {
        assert_eq!(asked_of(&json!({ "op": "moveOwner" })), Ok(Asked::MoveOwner));
        assert_eq!(asked_of(&json!({ "op": "shareNetwork", "network": "net_1" })), Ok(Asked::Share { network: "net_1".into(), level: Level::Append }));
        assert!(asked_of(&json!({ "op": "shareNetwork", "network": "net_1", "level": "god" })).is_err());
        assert!(asked_of(&json!({ "op": "unshareNetwork" })).is_err());
        assert!(asked_of(&json!({ "op": "take" })).is_err());
    }
}
