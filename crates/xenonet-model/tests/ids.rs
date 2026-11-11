//! The store's ids (`prj_`, `chk_`, `agt_`, `int_`, `spc_`) and what a
//! space is about (`EntityRef`): their known answers,
//! which another implementation can check itself against, and their
//! round trips. The known answers are test/fixtures/ids/known-answers.json;
//! after a change to how an id is made, write them again with
//!
//!   XENONET_WRITE_FIXTURES=1 npm run cargo -- test -p xenonet-model --test ids

use proptest::prelude::*;
use serde_json::{json, Value};
use xenonet_model::ids::{normalize_remote, AgentId, CheckoutId, EntityRef, Ids, IntentId, ProjectId, SpaceId, STORED_BYTES};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/ids/known-answers.json");

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |out, b| out + &format!("{b:02x}"))
}

fn answers() -> Value {
    let salt = [0x5a_u8; 32];
    let remotes = [
        "git@github.com:willctl/xenonet.git",
        "https://github.com/willctl/XenoNet",
        "ssh://git@github.com:22/willctl/xenonet",
        "https://user@github.com:443/willctl/xenonet.git",
        "ssh://git@gitlab.example:22/team/atlas.git",
        "ssh://git@gitlab.example:2222/team/atlas.git",
        "alice@gitlab.example:team/atlas",
    ];
    let projects: Vec<Value> = remotes
        .iter()
        .map(|r| {
            let key = normalize_remote(r);
            json!({ "remote": r, "repoKey": key, "id": ProjectId::of_repo(&salt, &key).to_string() })
        })
        .collect();
    let act = [0xa7_u8; 32];
    let ids = Ids::seeded("demo");
    let seeded = json!({
        "seed": "demo",
        "in order": [ids.project().to_string(), ids.checkout().to_string(), ids.agent().to_string(), ids.agent().to_string()],
    });
    let bytes: [u8; STORED_BYTES] = std::array::from_fn(|i| i as u8);
    let genesis = [0x3c_u8; 32];
    let about = [
        "prj_y5yxzivp2ggstrnktac5mae3mbk26xvx",
        "prs_aaaqeayeaudaocajbifqydiob4ibceqt",
        "xns1_aaaqeayeaudaocajbifqydiob4ibceqt",
        "actor:hermes@xns1_aaaqeayeaudaocajbifqydiob4ibceqt",
        "identity:github:github.com/willctl",
        "name:project:atlas",
    ];
    let about: Vec<Value> = about.iter().map(|a| json!({ "text": a, "kind": EntityRef::parse(a).expect("an entity").kind() })).collect();
    json!({
        "about": "The store's ids, as xenonet-model's ids.rs makes them (crates/xenonet-model/tests/ids.rs).",
        "projects": { "salt": hex(&salt), "of": projects },
        "intent": { "actHash": hex(&act), "id": IntentId::of_act(&act).to_string() },
        "bytes": { "of": hex(&bytes), "project": ProjectId::from_bytes(&bytes).to_string(), "checkout": CheckoutId::from_bytes(&bytes).to_string(), "agent": AgentId::from_bytes(&bytes).to_string() },
        "seeded": seeded,
        "space": { "genesisHash": hex(&genesis), "id": SpaceId::of_genesis(&genesis).to_string(), "ofBytes": SpaceId::from_bytes(&bytes).to_string() },
        "spaceAbout": about,
    })
}

#[test]
fn the_ids_give_their_known_answers() {
    let got = answers();
    if std::env::var("XENONET_WRITE_FIXTURES").as_deref() == Ok("1") {
        std::fs::create_dir_all(std::path::Path::new(FILE).parent().unwrap()).unwrap();
        std::fs::write(FILE, format!("{}\n", serde_json::to_string_pretty(&got).unwrap())).unwrap();
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    assert_eq!(got, want, "the known answers changed: if that's meant, write them again (see the top of this file)");
}

#[test]
fn one_repo_is_one_project_however_its_remote_is_written() {
    let salt = [7_u8; 32];
    let a = ProjectId::of_repo(&salt, &normalize_remote("git@github.com:willctl/xenonet.git"));
    let b = ProjectId::of_repo(&salt, &normalize_remote("https://user@github.com/willctl/XenoNet"));
    assert_eq!(a, b);
    assert_ne!(a, ProjectId::of_repo(&[8_u8; 32], &normalize_remote("git@github.com:willctl/xenonet.git")), "another person's salt, another id");
}

#[test]
fn a_seeded_source_repeats_and_a_random_one_doesnt() {
    let (a, b) = (Ids::seeded("x"), Ids::seeded("x"));
    assert_eq!((a.agent(), a.checkout()), (b.agent(), b.checkout()));
    assert_ne!(Ids::seeded("y").agent(), Ids::seeded("x").agent());
    let r = Ids::random();
    assert_ne!(r.agent(), r.agent());
}

#[test]
fn ids_are_their_kinds_and_nothing_else() {
    let p = ProjectId::from_bytes(&[0; STORED_BYTES]);
    assert_eq!(p.as_str(), "prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(CheckoutId::parse(p.as_str()).is_err(), "a project's id isn't a checkout's");
    for bad in [
        "prj_",
        "prj_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1",
        "prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "prj_0190fa00-0000-4000-8000-000000000001",
    ] {
        assert!(ProjectId::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(serde_json::to_value(&p).unwrap(), json!(p.as_str()));
    assert!(serde_json::from_value::<AgentId>(json!("agt_x")).is_err());
}

#[test]
fn a_space_is_its_genesis_and_nothing_else() {
    let a = SpaceId::of_genesis(&[1; 32]);
    assert_ne!(a, SpaceId::of_genesis(&[2; 32]));
    assert_eq!(SpaceId::parse(a.as_str()).unwrap(), a);
    assert!(ProjectId::parse(a.as_str()).is_err(), "a space's id isn't a project's");
}

#[test]
fn what_a_space_is_about_reads_back_and_refuses_the_rest() {
    let key = "xns1_aaaqeayeaudaocajbifqydiob4ibceqt";
    let pattern = regex::Regex::new(xenonet_model::ids::ENTITY_REF).unwrap();
    for good in [
        "prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        "prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        key.to_string(),
        format!("actor:grok-bot@{key}"),
        "identity:claude:acct:org".to_string(),
        "name:machine:local".to_string(),
        "name:project:a.b_c-d".to_string(),
    ] {
        let e = EntityRef::parse(&good).unwrap();
        assert_eq!(e.to_string(), good);
        assert_eq!(serde_json::from_value::<EntityRef>(serde_json::to_value(&e).unwrap()).unwrap(), e);
        assert_eq!(e.is_bound(), !good.starts_with("name:"));
        assert!(pattern.is_match(&good), "{good} matches the schema's pattern");
    }
    for bad in [
        "",
        "prj_x",
        "actor:hermes",
        "actor:Hermes@xns1_aaaqeayeaudaocajbifqydiob4ibceqt",
        "actor:hermes@prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "identity:",
        "identity:github:has space",
        "identity:Github:x",
        "name:project:",
        "name:project:a..b",
        "name:project:Atlas",
        "name:person:will",
        "name:project:.hidden",
        "project:atlas",
        "spc_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(EntityRef::parse(bad).is_err(), "{bad:?}");
    }
    assert!(serde_json::from_value::<EntityRef>(json!("name:project:a..b")).is_err());
}

proptest! {
    #[test]
    fn every_id_reads_back_as_it_was_written(bytes in proptest::array::uniform20(any::<u8>())) {
        let p = ProjectId::from_bytes(&bytes);
        prop_assert_eq!(ProjectId::parse(&p.to_string()).unwrap(), p.clone());
        prop_assert_eq!(serde_json::from_value::<ProjectId>(serde_json::to_value(&p).unwrap()).unwrap(), p);
        let c = CheckoutId::from_bytes(&bytes);
        prop_assert_eq!(c.as_str().parse::<CheckoutId>().unwrap(), c);
        let a = AgentId::from_bytes(&bytes);
        prop_assert_eq!(AgentId::parse(a.as_str()).unwrap(), a);
        let mut hash = [0_u8; 32];
        hash[..STORED_BYTES].copy_from_slice(&bytes);
        let i = IntentId::of_act(&hash);
        prop_assert_eq!(IntentId::parse(i.as_str()).unwrap(), i);
    }

    #[test]
    fn two_byte_strings_make_two_ids(a in proptest::array::uniform20(any::<u8>()), b in proptest::array::uniform20(any::<u8>())) {
        prop_assume!(a != b);
        prop_assert_ne!(AgentId::from_bytes(&a), AgentId::from_bytes(&b));
    }

    #[test]
    fn text_that_parses_is_written_back_the_same(s in "(prj|chk|agt|int|spc)_[a-z2-7]{32}") {
        let back = match &s[..4] {
            "prj_" => ProjectId::parse(&s).map(|i| i.to_string()),
            "chk_" => CheckoutId::parse(&s).map(|i| i.to_string()),
            "agt_" => AgentId::parse(&s).map(|i| i.to_string()),
            "spc_" => SpaceId::parse(&s).map(|i| i.to_string()),
            _ => IntentId::parse(&s).map(|i| i.to_string()),
        };
        prop_assert_eq!(back.unwrap(), s);
    }
}
