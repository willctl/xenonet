//! Xenolith's grant, as the window issues it (docs/design/authorization.md,
//! "Forward grants"): the person approves a forward grant for a context
//! key with their presence, the actor collects grants under it, and only
//! the one collected last forwards. Approving without presence, or from
//! anywhere but the window, is refused; taking it back ends it. The
//! person's device is a fake (xenonet-keys' `Fake`), never the machine's own.

use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_api::cores::{Cores, Grants, State, R};
use xenonet_api::wire::WireEnv;
use xenonet_authz::{Action, ActorGrant, ActorSlug, Binding, Schema, ACTOR_PREFIX};
use xenonet_runtime::grants::GrantsDriver;
use xenonet_runtime::runtime::{Runtime, Setup};

const ALPN: &str = "xenolith/sync/2";
const N1: &str = "net_00000000000000000000000000000001";
const RIGHTS: [&str; 3] = ["ctx-read", "ctx-append", "ctx-write"];

struct Empty;

#[async_trait::async_trait]
impl State for Empty {
    fn graph(&self) -> R<Option<Value>> {
        Ok(Some(json!({})))
    }
    fn nodes(&self) -> R<Vec<Value>> {
        Ok(Vec::new())
    }
    fn node(&self, _id: &str) -> R<Option<Value>> {
        Ok(None)
    }
}

/// The governance of a machine in no network.
struct Alone(Arc<xenonet_net::identity::Identity>);

impl xenonet_net::ground::Governance for Alone {
    fn facts(&self) -> xenonet_net::ground::Facts {
        let now = xenonet_text::time::now_ms() as i64;
        xenonet_net::ground::Facts { machine: xenonet_authz::MachineId::new("local").expect("a machine's id"), view: None, identity: self.0.clone(), synced_at: None, now }
    }
    fn identity(&self) -> Arc<xenonet_net::identity::Identity> {
        self.0.clone()
    }
    fn want_person(&self, _host: &str, _person: &str) {}
}

// A machine whose person's device says `yes` to every presence it's asked
// for, or no to every one.
fn world(name: &str, yes: bool) -> (Arc<Runtime>, Arc<xenonet_net::identity::Identity>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("xenonet-runtime-fwd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp folder");
    let state = dir.join("state").to_string_lossy().into_owned();
    let setup = Setup {
        demo: false,
        actors: true,
        port: 47996,
        home: dir.to_string_lossy().into_owned(),
        state_dir: state.clone(),
        projects_dir: dir.join("Projects").to_string_lossy().into_owned(),
        version: "0.0.0".into(),
        root: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("the checkout").to_path_buf(),
        runs_bin: None,
        permission_hold_ms: None,
        launcher_exec: None,
        made_up: xenonet_runtime::actors::MadeUp { sessions: xenonet_demo::session_titles(), claude_account: xenonet_demo::demo_claude_account },
        db: Some(Arc::new(xenonet_store::Db::open(std::path::Path::new(&state)).expect("the test machine's store"))),
    };
    let mut cores = Cores::pending(true, false);
    cores.state = Arc::new(Empty);
    xenonet_platforms::install();
    let w = xenonet_runtime::runtime(&cores, setup, &WireEnv::new("0.0.0")).expect("the area's parts");
    let identity = xenonet_net::identity::Identity::open(
        dir.join("state").join("identity"),
        Arc::new(xenonet_net::persist::Memory::default()),
        Some(Arc::new(xenonet_keys::Fake { allow: yes })),
    )
    .expect("an identity");
    xenonet_runtime::ground::set_governance(&w, Some(Arc::new(Alone(identity.clone()))));
    (w, identity, dir)
}

fn key(byte: u8) -> String {
    use xenonet_id::sign::Signer as _;
    xenonet_id::sign::SeedSigner::from_seed(&[byte; 32], xenonet_id::sign::KeyHome::File).expect("a key").public().id()
}

fn rights() -> Vec<String> {
    RIGHTS.iter().map(|r| (*r).to_string()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forward_grant_is_issued_with_presence_at_the_window_collected_renewed_and_taken_back() {
    let (w, identity, dir) = world("issue", true);
    let g = GrantsDriver(w.clone());
    let (lith, other) = (key(1), key(2));
    let nets = vec![N1.to_string()];
    // Not from the window: refused before anything is asked.
    let away = xenonet_runtime::grants::approve_forward(&w, false, "xenolith", ALPN, &lith, &nets, &rights(), "test");
    assert!(away.is_err_and(|e| e.contains("window")));
    assert!(g.approve_forward("xenolith", ALPN, &lith, &[], &rights(), "test").is_err(), "a grant reaches at least one network");
    assert_eq!(g.collect_forward("xenolith", ALPN, &lith), Err("unapproved"), "nothing approved yet");

    let approved = g.approve_forward("xenolith", ALPN, &lith, &nets, &rights(), "test").expect("approved with presence");
    assert_eq!((approved["key"].as_str(), approved["networks"].clone()), (Some(lith.as_str()), json!([N1])));
    assert_eq!(g.forward_grants()[0]["state"], "approved");
    assert_eq!(g.collect_forward("xenolith", ALPN, &other), Err("unapproved"), "another key collects nothing");
    assert_eq!(g.collect_forward("xenolith", "other/alpn/1", &lith), Err("unapproved"));

    let first = g.collect_forward("xenolith", ALPN, &lith).expect("collected");
    let f = g.forward(&first, ALPN).expect("it forwards");
    assert_eq!((f.actor.as_str(), f.networks.clone()), ("xenolith", Some(nets.clone())));
    assert!(f.until - xenonet_text::time::now_ms() as i64 > 29 * 86_400_000, "thirty days");
    assert_eq!(g.forward_grants()[0]["state"], "granted");
    assert!(!g.forward_grants().to_string().contains(&first), "the window's list holds no grant");
    let db = w.store.with(|s| s.db().clone());
    assert!(db.context_key(&lith, xenonet_text::time::now_ms() as i64).unwrap(), "the context key is recorded for the map");

    // A renewal: the new one forwards, the one before is revoked.
    let second = g.collect_forward("xenolith", ALPN, &lith).expect("renewed");
    assert!(g.forward(&second, ALPN).is_ok());
    assert_eq!(g.forward(&first, ALPN).err(), Some("revoked"));

    // A grant for the same key minted outside the approval forwards nothing.
    let facts = xenonet_runtime::ground::governance(&w).unwrap().facts();
    let root = identity.token_root(&facts.machine, facts.now).unwrap();
    let loose = ActorGrant { actor: ActorSlug::new("xenolith").unwrap(), network: None, rights: vec![Action::CtxRead], until: facts.now + 60_000 };
    let loose = root.mint_actor_with(&loose, &Schema::XENONET, &Binding::new(Some(&lith), Some(ALPN)).unwrap()).unwrap().encode(ACTOR_PREFIX).unwrap();
    assert_eq!(g.forward(&loose, ALPN).err(), Some("no-grant"));

    // Away past its grant's end: no renewal without the person.
    let (id, ids, _) = db.forward_grant("xenolith").unwrap().unwrap().token.unwrap();
    db.set_forward_token("xenolith", &id, &ids, facts.now - 1).unwrap();
    assert_eq!(g.collect_forward("xenolith", ALPN, &lith), Err("lapsed"));
    assert_eq!(g.forward_grants()[0]["state"], "lapsed");
    db.set_forward_token("xenolith", &id, &ids, facts.now + 60_000).unwrap();

    // Taken back: the grant is revoked, and nothing more is collected.
    let gone = g.revoke_forward("xenolith").unwrap();
    assert_eq!(gone.map(|v| v["actor"].clone()), Some(json!("xenolith")));
    assert_eq!(g.forward(&second, ALPN).err(), Some("revoked"));
    assert_eq!(g.collect_forward("xenolith", ALPN, &lith), Err("unapproved"));
    assert_eq!(g.forward_grants(), json!([]));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forward_grant_without_the_persons_presence_is_refused() {
    let (w, _, dir) = world("no-presence", false);
    let g = GrantsDriver(w);
    let refused = g.approve_forward("xenolith", ALPN, &key(1), &[N1.to_string()], &rights(), "test");
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(g.forward_grants(), json!([]), "nothing approved");
    assert_eq!(g.collect_forward("xenolith", ALPN, &key(1)), Err("unapproved"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_presence_for_a_forward_grant_names_its_key_alpn_and_networks() {
    let base = xenonet_runtime::grants::forward_scopes(&key(1), ALPN, &[N1.to_string()]).unwrap();
    assert_eq!(base, xenonet_runtime::grants::forward_scopes(&key(1), ALPN, &[N1.to_string(), N1.to_string()]).unwrap(), "a network once");
    assert_ne!(base, xenonet_runtime::grants::forward_scopes(&key(2), ALPN, &[N1.to_string()]).unwrap());
    assert_ne!(base, xenonet_runtime::grants::forward_scopes(&key(1), "other/alpn/1", &[N1.to_string()]).unwrap());
    assert_ne!(base, xenonet_runtime::grants::forward_scopes(&key(1), ALPN, &[N1.to_string(), "net_00000000000000000000000000000002".into()]).unwrap());
}
