//! The integrations (crates/xenonet-integrations): their settings, the
//! history and Pi's records, the routing preview, and OpenShell's sandbox
//! jobs with their text reviews. The decisions are the integrations
//! crate's; what its sandbox and review actions wait on (commands, git
//! runs, records) is done here. The demo's decider, OpenShell and records
//! are made up.
//!
//! OpenShell's and the reviews' state is shared between their actions, so
//! it lives on a thread of its own (fx::local), one per integrations, and
//! each action runs there.
//!
//! A route goes through JevRouter's rules as jev.rs has them, where the JS
//! had the npm library.

use crate::decide::Decisions;
use crate::fx::{self, now_ms, Locked, RunOptions};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;
use xenonet_api::cores::{CoreError, Decider, Integrations, Kind, State, R};
use xenonet_integrations::common::{Fail, Records, Tickets};
use xenonet_integrations::openshell::ShellHost;
use xenonet_integrations::reviews::{Git, ReviewHost};
use xenonet_integrations::{index, openshell, pi, reviews, routing};
use xenonet_os::error::Failure;

fn fail(f: Fail) -> CoreError {
    if f.status == 0 {
        let mut e = CoreError::new(f.message);
        e.code = (!f.code.is_empty()).then_some(f.code);
        return e;
    }
    CoreError::of(Kind::Integration, f.status, &f.code, f.message)
}

// What an action failed with: an integration's refusal, or a system error.
fn thrown(e: Failure) -> CoreError {
    if e.fields.get("integration") == Some(&json!(true)) {
        let status = e.fields.get("status").and_then(Value::as_u64).map_or(500, |s| s as u16);
        let code = e.code().unwrap_or("failed").to_string();
        return CoreError::of(Kind::Integration, status, &code, e.message);
    }
    let code = e.fields.get("code").map(xenonet_text::text::display);
    let mut c = CoreError::new(e.message);
    c.code = code;
    c
}

// A record's failure, as the shell reads it.
fn record_failure(f: Fail) -> Failure {
    let mut m = Map::new();
    if !f.code.is_empty() {
        m.insert("code".into(), json!(f.code));
    }
    if f.status != 0 {
        m.insert("status".into(), json!(f.status));
        m.insert("integration".into(), json!(true));
    }
    Failure::with(f.message, m)
}

// Each integrations' OpenShell and reviews, on their thread.
struct Local {
    shell: openshell::Shared,
    reviews: reviews::Shared,
}

thread_local! {
    static LOCAL: RefCell<HashMap<u64, Local>> = RefCell::new(HashMap::new());
}

fn local(id: u64, platform: &str, collect: &str, reviews_dir: Option<String>, demo: bool) -> (openshell::Shared, reviews::Shared) {
    LOCAL.with(|l| {
        let mut l = l.borrow_mut();
        let made = l.entry(id).or_insert_with(|| Local {
            shell: Rc::new(RefCell::new(openshell::OpenShell {
                tickets: Tickets::default(),
                platform: platform.into(),
                system_root: fx::env_set("SystemRoot"),
                collect_text: collect.into(),
            })),
            reviews: Rc::new(RefCell::new(reviews::Reviews::new(reviews_dir, demo))),
        });
        (made.shell.clone(), made.reviews.clone())
    })
}

pub struct Parts {
    /// The state folder's integrations/, or None in the demo.
    pub dir: Option<String>,
    pub home: String,
    pub demo: bool,
    pub decider: Arc<Decisions>,
    pub state: Arc<dyn State>,
    /// XenoNet's own folder, where reviews.js has the program that collects
    /// text inside a sandbox.
    pub root: String,
}

pub struct Integrated {
    id: u64,
    parts: Parts,
    records: Arc<Mutex<Records>>,
    settings_file: Option<String>,
    pi_on: Mutex<bool>,
    tickets: Mutex<Tickets>,
    routing_busy: Mutex<bool>,
    seeded: OnceCell<()>,
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Integrated {
    pub fn new(parts: Parts) -> R<Arc<Self>> {
        let records_dir = parts.dir.as_ref().map(|d| Path::new(d).join("records").to_string_lossy().into_owned());
        let records = Records::new(records_dir, 1000).map_err(fail)?;
        let settings_file = parts.dir.as_ref().map(|d| Path::new(d).join("settings.json").to_string_lossy().into_owned());
        let pi_on = index::pi_on(settings_file.as_deref());
        Ok(Arc::new(Integrated {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            parts,
            records: Arc::new(Mutex::new(records)),
            settings_file,
            pi_on: Mutex::new(pi_on),
            tickets: Mutex::new(Tickets::default()),
            routing_busy: Mutex::new(false),
            seeded: OnceCell::new(),
        }))
    }

    fn reviews_dir(&self) -> Option<String> {
        self.parts.dir.as_ref().map(|d| Path::new(d).join("reviews").to_string_lossy().into_owned())
    }

    fn records(&self) -> R<Vec<Value>> {
        self.records.locked().list().map_err(fail)
    }

    // COLLECT_TEXT: the program that collects text inside a sandbox, which
    // runs on the sandbox's Node, as reviews.js makes it from its own
    // functions; the binary carries it.
    fn collect_text() -> String {
        reviews::COLLECT_TEXT.to_string()
    }

    fn shared(&self, collect: String) -> impl FnOnce() -> (openshell::Shared, reviews::Shared) + Send + 'static {
        let (id, demo, dir) = (self.id, self.parts.demo, self.reviews_dir());
        move || local(id, fx::platform(), &collect, dir, demo)
    }

    // The demo's made-up decision, export and sandboxes, once.
    async fn ready(self: &Arc<Self>) -> R<()> {
        if !self.parts.demo {
            return Ok(());
        }
        let me = self.clone();
        self.seeded
            .get_or_try_init(|| async move {
                let made = index::demo_records();
                {
                    let mut r = me.records.locked();
                    r.add(&made["decision"], now_ms()).map_err(fail)?;
                    r.add(&made["export"], now_ms()).map_err(fail)?;
                }
                let collect = Self::collect_text();
                for pair in made["sandboxes"].as_array().cloned().unwrap_or_default() {
                    let (name, options) = (xenonet_text::text::display(&pair[0]), pair[1].clone());
                    let shared = me.shared(collect.clone());
                    let o = options.clone();
                    let seed = fx::local(move || async move {
                        let (_, r) = shared();
                        let seeded = r.borrow_mut().seed(&o, now_ms());
                        seeded.map_err(|f| Failure::msg(f.message))
                    })
                    .await
                    .unwrap_or_else(|| Err(Failure::msg("the demo's sandboxes stopped before they were made")))
                    .map_err(thrown)?;
                    let sandbox = index::demo_sandbox(&name, &options, &seed);
                    me.records.locked().add(&sandbox, now_ms()).map_err(fail)?;
                }
                Ok::<(), CoreError>(())
            })
            .await?;
        Ok(())
    }

    async fn shell(self: &Arc<Self>, method: &str, arg: Value, arg2: Value) -> R<Value> {
        let shared = self.shared(Self::collect_text());
        let (me, method, rt) = (self.clone(), method.to_string(), tokio::runtime::Handle::current());
        let done = fx::local(move || async move {
            let (sh, reviews) = shared();
            let host = Here { me, reviews, rt };
            match method.as_str() {
                "status" => openshell::status(&host, &sh, &arg).await,
                "preview" => openshell::preview(&host, &sh, &arg).await,
                "launch" => openshell::launch(&host, &sh, &arg).await,
                "inspect" => openshell::inspect(&host, &sh, &arg).await,
                "review" => openshell::review(&host, &arg).await,
                "retrieve" => openshell::retrieve(&host, &sh, &arg).await,
                "exportReview" => openshell::export_review(&host, &arg, &arg2).await,
                "forgetReview" => openshell::forget_review(&host, &arg).await,
                "remove" => openshell::remove(&host, &sh, &arg).await,
                other => Err(Failure::msg(format!("integrations.shell.{other} is not a function"))),
            }
        })
        .await;
        done.unwrap_or_else(|| Err(Failure::msg("an action stopped before it was done"))).map_err(thrown)
    }

    // A route: the preview's task to the decider, its answer checked and
    // made a decision by JevRouter's rules (jev.rs), and the record kept.
    async fn route(&self, ticket: &Value) -> R<Value> {
        let held = self.tickets.locked().take(&xenonet_text::text::display(ticket), now_ms()).map_err(fail)?;
        if xenonet_text::json::stringify(&held["candidates"], 0) != xenonet_text::json::stringify(&self.inventory(), 0) {
            return Err(CoreError::of(Kind::Integration, 409, "changed", "The available agents changed. Preview again."));
        }
        let request = xenonet_text::text::display(&held["request"]);
        let candidates = held["candidates"].as_array().cloned().unwrap_or_default();
        let mut metrics = json!({ "model": self.model(), "cost": 0, "ms": 0 });
        let payload = routing::payload(&request, &candidates);
        let decided = if self.parts.demo { Ok(index::demo_decision()) } else { self.parts.decider.decide_on("routing", &payload["state"], &payload["questions"]).await };
        let raw = match decided {
            Ok(response) => {
                metrics = response.clone();
                if routing::answer_ok(&response["answers"], &candidates) {
                    Ok(json!({ "answers": response["answers"], "model": response["model"] }))
                } else {
                    Err(crate::jev::JevError::http("The provider returned an invalid probability distribution."))
                }
            }
            Err(e) => Err(crate::jev::JevError::http(e.message)),
        };
        let result = crate::jev::route(&candidates, &["suggest".to_string()], raw);
        self.records.locked().add(&routing::decision_record(&result, &request, &metrics), now_ms()).map_err(fail)
    }

    fn inventory(&self) -> Value {
        routing::inventory(&self.parts.state.nodes().unwrap_or_default())
    }

    async fn decider_status(&self) -> R<Value> {
        if self.parts.demo {
            return Ok(json!({ "key": "demo", "model": "demo/jev" }));
        }
        self.parts.decider.status().await
    }

    fn model(&self) -> Value {
        json!(if self.parts.demo { "demo/jev" } else { self.parts.decider.model() })
    }
}

// git as the reviews ask for it; `allow_exit` is an exit code that still
// answers (check-ignore's 1, for nothing ignored).
async fn git(args: Vec<String>, env: Vec<(String, String)>, timeout_ms: f64, input: Option<String>, allow_exit: Option<i64>) -> Result<String, Failure> {
    let r = fx::exec_file("git", &args, timeout_ms as u64, RunOptions { env: Some(env), input, ..Default::default() }).await;
    if r.ok() || allow_exit.is_some_and(|a| json!(a) == r.code) {
        return Ok(r.stdout);
    }
    Err(Failure::from_value(&json!({ "message": format!("Command failed: git {}\n{}", args.join(" "), r.stderr), "code": r.code, "stderr": r.stderr, "stdout": r.stdout })))
}

/// What the shell's and the reviews' actions ask of the service, on their
/// thread; commands and git run on the service's runtime.
struct Here {
    me: Arc<Integrated>,
    reviews: reviews::Shared,
    rt: tokio::runtime::Handle,
}

impl Here {
    async fn on<T: Send + 'static>(&self, f: impl std::future::Future<Output = Result<T, Failure>> + Send + 'static) -> Result<T, Failure> {
        self.rt.spawn(f).await.unwrap_or_else(|e| Err(Failure::msg(format!("a request failed: {e}"))))
    }
}

#[async_trait(?Send)]
impl ReviewHost for Here {
    async fn now(&self) -> f64 {
        now_ms()
    }
    async fn git(&self, run: &Git) -> Result<String, Failure> {
        let (args, env, timeout, input, allow) = (run.args.clone(), fx::env_pairs(&run.env), run.timeout_ms, run.input.clone(), run.allow_exit);
        self.on(git(args, env, timeout, input, allow)).await
    }
    async fn snapshot(&self, source: &Value) -> Result<Value, Failure> {
        reviews::source_snapshot(self, &xenonet_text::text::display(source), &fx::env_map()).await
    }
}

#[async_trait(?Send)]
impl ShellHost for Here {
    async fn run(&self, file: &str, args: &[String], timeout_ms: f64) -> Result<Value, Failure> {
        if self.me.parts.demo {
            let args: Vec<Value> = args.iter().map(|a| json!(a)).collect();
            return index::demo_run(&args).map(|out| json!({ "stdout": out })).map_err(Failure::msg);
        }
        let (file, args) = (file.to_string(), args.to_vec());
        self.on(async move {
            let mut env: Vec<(String, String)> = std::env::vars().collect();
            env.push(("NO_COLOR".into(), "1".into()));
            let r = fx::exec_file(&file, &args, timeout_ms as u64, RunOptions { env: Some(env), ..Default::default() }).await;
            if r.ok() {
                Ok(json!({ "stdout": r.stdout, "stderr": r.stderr }))
            } else {
                Err(Failure::from_value(
                    &json!({ "message": format!("Command failed: {file} {}\n{}", args.join(" "), r.stderr), "code": r.code, "stderr": r.stderr, "stdout": r.stdout, "killed": r.timed_out }),
                ))
            }
        })
        .await
    }
    async fn records_list(&self) -> Result<Value, Failure> {
        self.me.records.locked().list().map(Value::Array).map_err(record_failure)
    }
    async fn records_add(&self, value: &Value) -> Result<Value, Failure> {
        self.me.records.locked().add(value, now_ms()).map_err(record_failure)
    }
    async fn records_update(&self, id: &Value, fields: &Value) -> Result<Value, Failure> {
        self.me.records.locked().update(id, fields).map_err(record_failure)
    }
    fn reviews(&self) -> reviews::Shared {
        self.reviews.clone()
    }
    fn env(&self) -> Map<String, Value> {
        fx::env_map()
    }
}

/// The Integrations trait: the method each route calls by its JS name.
pub struct IntegrationsCore(pub Arc<Integrated>);

#[async_trait]
impl Integrations for IntegrationsCore {
    async fn call(&self, method: &str, args: Vec<Value>) -> R<Value> {
        let me = &self.0;
        me.ready().await?;
        let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
        match method {
            "status" => {
                let profiles = if me.parts.demo {
                    json!([{ "backend": if fx::platform() == "win32" { "wsl" } else { "native" }, "distro": "Demo-Linux" }])
                } else if fx::platform() != "win32" {
                    json!([{ "backend": "native", "distro": "" }])
                } else {
                    let r = fx::exec_file(&openshell::wsl_file(fx::env_set("SystemRoot").as_deref()), &["--list".into(), "--quiet".into()], 15000, RunOptions::default()).await;
                    if r.ok() {
                        openshell::wsl_profiles(&r.stdout)
                    } else {
                        json!([])
                    }
                };
                Ok(
                    json!({ "decision": me.decider_status().await?, "piOn": *me.pi_on.locked(), "piRoot": index::pi_root_shown(&me.parts.home), "profiles": profiles, "platform": fx::platform(), "demo": me.parts.demo }),
                )
            }
            "history" => Ok(index::history(&me.records()?, &arg(0))),
            "pi" => {
                let on = *me.pi_on.locked();
                let source = if on && !me.parts.demo {
                    let root = index::pi_root(&me.parts.home);
                    tokio::task::spawn_blocking(move || pi::scan(&root, 500, 8 * 1024 * 1024)).await.map_err(|e| CoreError::new(e.to_string()))?
                } else {
                    json!({ "records": [], "warnings": [] })
                };
                Ok(index::pi(&me.records()?, &source, on, &arg(0)))
            }
            "enablePi" => {
                let r = index::set_pi(me.settings_file.as_deref(), &arg(0)).map_err(fail)?;
                *me.pi_on.locked() = r.get("piOn") == Some(&json!(true));
                Ok(r)
            }
            "importPi" => {
                let plan = index::import_plan(&me.records()?, &arg(0)).map_err(fail)?;
                let mut r = me.records.locked();
                if plan.get("add").is_some_and(|a| !a.is_null()) {
                    r.add(&plan["add"], now_ms()).map_err(fail)
                } else {
                    r.update(&plan["update"], &plan["fields"]).map_err(fail)
                }
            }
            "routing.preview" => {
                let request = routing::request_of(Some(&arg(0))).map_err(fail)?;
                let candidates = me.inventory();
                let list = candidates.as_array().cloned().unwrap_or_default();
                let mut out = me.tickets.locked().issue(json!({ "request": request, "candidates": candidates }), now_ms()).map_err(fail)?;
                out["outgoing"] = routing::outgoing(&me.model(), &request, &list);
                out["candidates"] = candidates;
                out["note"] = json!(routing::NOTE);
                Ok(out)
            }
            "routing.route" => {
                {
                    let mut busy = me.routing_busy.locked();
                    if *busy {
                        return Err(CoreError::of(Kind::Integration, 409, "busy", "A routing decision is already running."));
                    }
                    *busy = true;
                }
                let out = me.route(&arg(0)).await;
                *me.routing_busy.locked() = false;
                out
            }
            "routing.outcome" => {
                routing::outcome_ok(&arg(1)).map_err(fail)?;
                let id = arg(0);
                let is_decision = me.records()?.iter().find(|r| r["id"] == id).is_some_and(|r| r["kind"] == "decision");
                if !is_decision {
                    return Err(CoreError::of(Kind::Integration, 404, "missing", "No such decision."));
                }
                me.records.locked().update(&id, &json!({ "outcome": arg(1) })).map_err(fail)
            }
            m if m.starts_with("shell.") => me.shell(&m["shell.".len()..], arg(0), arg(1)).await,
            other => Err(CoreError::new(format!("integrations.{other} is not a function"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::Secrets;
    use xenonet_api::cores::Pending;
    use xenonet_api::wire::Emitter;

    fn made(demo: bool, dir: Option<String>) -> IntegrationsCore {
        let decider = Arc::new(Decisions::new(Arc::new(Secrets::memory()), Emitter::new()));
        let root = std::env::current_dir().ok().and_then(|d| d.ancestors().nth(2).map(|p| p.to_string_lossy().into_owned())).unwrap_or_default();
        IntegrationsCore(Integrated::new(Parts { dir, home: "/home/nova".into(), demo, decider, state: Arc::new(Pending), root }).unwrap())
    }

    // test/integrations.test.js's demo cases: its made-up records, the
    // status, and the preview of a route.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_demos_integrations() {
        let i = made(true, None);
        let st = i.call("status", vec![]).await.unwrap();
        assert_eq!((st["demo"].clone(), st["decision"]["model"].clone()), (json!(true), json!("demo/jev")));
        let h = i.call("history", vec![json!({})]).await.unwrap();
        assert!(h.to_string().contains("decision"), "{h}");
        assert!(i.call("routing.preview", vec![json!({})]).await.is_err(), "a route needs a task");
        let p = i.call("routing.preview", vec![json!({ "request": "Fix the flaky test in atlas-api" })]).await.unwrap();
        assert!(p["ticket"].is_string() && p["candidates"].is_array());
        let routed = i.call("routing.route", vec![p["ticket"].clone()]).await.unwrap();
        assert_eq!(routed["kind"], "decision");
        assert!(routed["candidates"].as_array().is_some_and(|c| !c.is_empty()));
        assert!(i.call("routing.route", vec![p["ticket"].clone()]).await.unwrap_err().is(Kind::Integration), "a ticket works once");
        assert_eq!(i.call("routing.outcome", vec![json!("nope"), json!("accepted")]).await.unwrap_err().status, Some(404));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pi_turned_on_and_off_in_its_settings() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-intg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let i = made(false, Some(dir.to_string_lossy().into_owned()));
        assert_eq!(i.call("enablePi", vec![json!(true)]).await.unwrap()["piOn"], true);
        assert_eq!(i.call("status", vec![]).await.unwrap()["piOn"], true);
        assert_eq!(i.call("enablePi", vec![json!(false)]).await.unwrap()["piOn"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
