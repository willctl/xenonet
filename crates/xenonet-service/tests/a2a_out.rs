// Outbound A2A actors, through the service's side (a2a/outbound.rs),
// against the fake A2A agent xenonet-a2a's tests run (its tests/fake):
// the person registers one with presence over exactly what they approved,
// sessions hand it work as xenonet-authz decides, and what comes back is
// the task as it stands, framed as information, with the bearer nowhere.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

#[path = "../../../adapters/a2a/tests/fake/mod.rs"]
#[expect(dead_code, reason = "the client's own helpers there are for its tests")]
mod fake;

use async_trait::async_trait;
use fake::{fake, roots, Fake, TOKEN};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_authz::Attestation;
use xenonet_service::a2a::outbound::{Caller, Keep, Outbound, OutboundParts, Register};

const NOW: f64 = 1_800_000_000_000.0;

#[derive(Default)]
struct Memory(Mutex<HashMap<String, String>>);

#[async_trait]
impl Keep for Memory {
    async fn get(&self, target: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(target).cloned())
    }
    async fn set(&self, target: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(target.into(), secret.into());
        Ok(())
    }
    async fn remove(&self, target: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(target);
        Ok(())
    }
}

type Asked = Arc<Mutex<Vec<(String, Value)>>>;

fn outbound() -> (Arc<Outbound>, Asked, Arc<Memory>) {
    let asked: Asked = Arc::default();
    let a = asked.clone();
    let keep = Arc::new(Memory::default());
    let o = Outbound::new(OutboundParts {
        db: None,
        keep: keep.clone(),
        present: Arc::new(move |slug: &str, scopes: &Value, _reason: &str| {
            a.lock().unwrap().push((slug.to_string(), scopes.clone()));
            Ok(())
        }),
        now: Arc::new(|| NOW),
        block: Duration::from_secs(5),
        roots: Some(roots()),
    })
    .unwrap();
    (o, asked, keep)
}

fn session(id: &str, project: Option<&str>) -> Caller {
    Caller { agent: id.into(), attestation: Attestation::ProcessChecked, project: project.map(str::to_string) }
}

async fn register(o: &Outbound, f: &Fake, approve_host: Option<String>) -> Result<Value, String> {
    let private = format!("localhost:{}", f.port);
    let shown = o.preview(&f.url("/"), Some(&private)).await?;
    o.register(Register {
        slug: "scout".into(),
        card: f.url("/"),
        hash: shown["hash"].as_str().unwrap().into(),
        approve_host,
        private_host: Some(private),
        agents: json!(["claude:fix"]),
        projects: json!(["atlas"]),
        days: 30.0,
        token: Some(TOKEN.into()),
    })
    .await
}

#[tokio::test]
async fn the_person_registers_one_with_presence_over_what_they_saw() {
    let f = fake();
    let (o, asked, keep) = outbound();
    let shown = o.preview(&f.url("/"), Some(&format!("localhost:{}", f.port))).await.unwrap();
    assert_eq!((shown["name"].as_str(), shown["sameHost"].as_bool()), (Some("Scout"), Some(true)));
    let status = register(&o, &f, None).await.unwrap();
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "one presence");
    assert_eq!(asked[0].0, "scout");
    assert_eq!(asked[0].1["card"], shown["hash"], "the presence covers the pin the window showed");
    assert_eq!(asked[0].1["agents"], json!(["claude:fix"]));
    assert_eq!(status["outbound"][0]["id"], "actor:scout");
    assert!(!status.to_string().contains(TOKEN), "the page never gets the bearer");
    assert_eq!(keep.0.lock().unwrap().get("XenoNet/A2A agent/scout").map(String::as_str), Some(TOKEN), "it's in the credential store");
    // A card that changed since the window showed it isn't registered.
    f.seen.lock().unwrap().version = "2.0.0".into();
    let e = o
        .register(Register {
            slug: "scout".into(),
            card: f.url("/"),
            hash: shown["hash"].as_str().unwrap().into(),
            private_host: Some(format!("localhost:{}", f.port)),
            agents: json!("all"),
            days: 1.0,
            ..Register::default()
        })
        .await
        .unwrap_err();
    assert!(e.contains("changed"), "{e}");
}

#[tokio::test]
async fn an_interface_on_another_host_is_registered_only_when_approved() {
    let f = fake();
    f.seen.lock().unwrap().interface = Some(format!("127.0.0.1:{}", f.port));
    let (o, asked, _) = outbound();
    let e = register(&o, &f, None).await.unwrap_err();
    assert!(e.contains("another host"), "{e}");
    assert!(asked.lock().unwrap().is_empty(), "nothing was asked of the person");
    register(&o, &f, Some(format!("127.0.0.1:{}", f.port))).await.unwrap();
    assert_eq!(asked.lock().unwrap()[0].1["host"], format!("127.0.0.1:{}", f.port));
}

#[tokio::test]
async fn sessions_in_the_scope_hand_it_work() {
    let f = fake();
    let (o, _, _) = outbound();
    register(&o, &f, None).await.unwrap();
    let a = o.delegate(&session("claude:fix", None), "actor:scout", Some("hello"), None).await.unwrap();
    assert_eq!((a["state"].as_str(), a["text"].as_str()), (Some("done"), Some("the answer")), "{a}");
    assert!(a["note"].as_str().unwrap().contains("approves nothing"));
    let a = o.delegate(&session("codex:other", Some("atlas")), "scout", Some("hello"), None).await.unwrap();
    assert_eq!(a["state"], "done", "by its project");
    assert_eq!(f.seen.lock().unwrap().auth, vec![format!("Bearer {TOKEN}"); 2], "the remote's bearer, and nothing of XenoNet's");
}

#[tokio::test]
async fn sessions_outside_it_are_refused() {
    let f = fake();
    let (o, _, _) = outbound();
    register(&o, &f, None).await.unwrap();
    let e = o.delegate(&session("codex:other", Some("billing")), "actor:scout", Some("hello"), None).await.unwrap_err();
    assert!(e.contains("didn't let codex:other"), "{e}");
    let unchecked = Caller { attestation: Attestation::Unchecked, ..session("claude:fix", None) };
    let e = o.delegate(&unchecked, "actor:scout", Some("hello"), None).await.unwrap_err();
    assert!(e.contains("unchecked"), "{e}");
    let e = o.delegate(&session("claude:fix", None), "actor:nobody", Some("hello"), None).await.unwrap_err();
    assert!(e.contains("no outbound actor"), "{e}");
    assert!(f.seen.lock().unwrap().methods.is_empty(), "nothing went out");
}

#[tokio::test]
async fn a_question_goes_back_and_its_answer_goes_on_in_the_task() {
    let f = fake();
    let (o, _, _) = outbound();
    register(&o, &f, None).await.unwrap();
    let me = session("claude:fix", None);
    let a = o.delegate(&me, "actor:scout", Some("more"), None).await.unwrap();
    assert_eq!((a["state"].as_str(), a["text"].as_str()), (Some("asks-you"), Some("Which branch?")));
    let id = a["delegation"].as_str().unwrap().to_string();
    let b = o.delegate(&me, "actor:scout", Some("main"), Some(&id)).await.unwrap();
    assert_eq!((b["state"].as_str(), b["text"].as_str(), b["delegation"].as_str()), (Some("done"), Some("went on with main"), Some(id.as_str())));
    // Another session can't read or answer it.
    let e = o.delegate(&session("codex:other", Some("atlas")), "actor:scout", None, Some(&id)).await.unwrap_err();
    assert!(e.contains("isn't a delegation of yours"), "{e}");
}

#[tokio::test]
async fn the_bearer_is_never_in_what_a_session_gets() {
    let f = fake();
    let (o, _, _) = outbound();
    register(&o, &f, None).await.unwrap();
    let me = session("claude:fix", None);
    let a = o.delegate(&me, "actor:scout", Some("fail"), None).await.unwrap();
    assert_eq!(a["state"], "failed");
    assert!(!a.to_string().contains(TOKEN) && a["text"].as_str().unwrap().contains("[token]"), "{a}");
    let a = o.delegate(&me, "actor:scout", Some("echo"), None).await.unwrap();
    assert_eq!(a["state"], "failed");
    assert!(!a.to_string().contains(TOKEN), "{a}");
    let a = o.delegate(&me, "actor:scout", Some("sign in"), None).await.unwrap();
    assert_eq!(a["state"], "asks-person");
    assert!(a["next"].as_str().unwrap().contains("sends no credential"));
    assert!(!o.status().to_string().contains(TOKEN));
}

#[tokio::test]
async fn removing_one_takes_its_bearer() {
    let f = fake();
    let (o, _, keep) = outbound();
    register(&o, &f, None).await.unwrap();
    assert!(o.remove("actor:scout").await.unwrap());
    assert!(keep.0.lock().unwrap().is_empty());
    assert!(o.delegate(&session("claude:fix", None), "actor:scout", Some("hello"), None).await.is_err());
}
