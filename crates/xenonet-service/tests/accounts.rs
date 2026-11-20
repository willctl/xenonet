// Several accounts per app, end to end: a project's rule names the Claude
// Code account its sessions run as, and what a start does about it (the
// folder it runs in, the sign-in it is not given, the commands it is kept
// from) must hold for every later turn of the session too: a wake, a copy.
// One service in this process, with the fake CLI keeping each run's
// arguments and the names of its environment, as in net_sandbox.rs.

#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]
#![expect(dead_code, reason = "the shared engine harness does more than this file uses")]

mod common;
#[path = "common/engine.rs"]
mod engine;
mod ports;

use engine::*;

use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use xenonet_service::compose::runtime::{accounts, delivery, sandbox};

fn root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("xenonet-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

const TOKEN: &str = "made-up-token-for-the-ledger-run";

/// What the service's own environment holds on this machine: a key and a
/// base URL of a provider, which outrank a folder's sign-in if a run keeps
/// them.
fn service_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> =
        ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL"].iter().map(|k| ((*k).to_string(), "not-for-the-run".to_string())).collect();
    env.push(("PATH".into(), std::env::var("PATH").unwrap_or_default()));
    env
}

/// Nova's machine: atlas is ruled to her default folder's account, ledger to
/// her work folder's; both hold a made-up sign-in.
struct Desk {
    host: Host,
    atlas: String,
    ledger: String,
    work: String,
    rules: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    root: PathBuf,
}

impl Drop for Desk {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn desk(name: &str, atlas_rule: Value, sessions: impl FnOnce(&str, &str) -> Vec<Sess>) -> Desk {
    let root = root(name);
    let home = root.join("desk").join("home");
    let (atlas, ledger, work) = (home.join("atlas"), home.join("ledger"), home.join(".claude-work"));
    for d in [&atlas, &ledger, &work] {
        std::fs::create_dir_all(d).unwrap();
    }
    let (atlas, ledger, work) = (atlas.to_string_lossy().into_owned(), ledger.to_string_lossy().into_owned(), work.to_string_lossy().into_owned());
    let accounts = Accounts {
        rules: std::sync::Arc::new(std::sync::Mutex::new(vec![(atlas.clone(), atlas_rule), (ledger.clone(), json!({ "claude": "nova-work@example.com" }))])),
        roots: vec![(None, Some("nova@example.com".into())), (Some(work.clone()), Some("nova-work@example.com".into()))],
        answers: true,
        limits: json!({ "claude": { "fiveHour": 40 }, format!("claude@{work}"): { "fiveHour": 92 } }),
    };
    let rules = accounts.rules.clone();
    // A folder a start names is one the map knows: here, by a session that ran in it.
    let scratch = home.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let scratch = scratch.to_string_lossy().into_owned();
    let mut known = vec![
        sess("claude:0190d3e0-0000-7000-8000-0000000a0001", "Atlas", &atlas),
        sess("claude:0190d3e0-0000-7000-8000-0000000a0002", "Ledger", &ledger),
        sess("claude:0190d3e0-0000-7000-8000-0000000a0003", "Scratch", &scratch),
    ];
    known.extend(sessions(&atlas, &work));
    let host = Host::open_accounts("desk", root.join("desk"), known, None, false, accounts).await;
    // A start is made by someone: the person, at a network they founded.
    let (st, v) = host.window("POST", "/api/net/create", json!({ "name": "home", "host": "desk" })).await;
    assert_eq!(st, 200, "{v}");
    host.world.starts.set_sign_in(TOKEN);
    host.world.starts.set_environment(service_env());
    Desk { host, atlas, ledger, work, rules, root }
}

/// The `n`th run's sign-in descriptor and the names of its environment.
async fn env_of(h: &Host, n: usize) -> Vec<String> {
    let end = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(f) = fake_files(&h.dir, n).filter(|f| !f.1.is_empty()) {
            return f.1;
        }
        assert!(std::time::Instant::now() < end, "waited for run {n}'s environment");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn fd_token(h: &Host, n: usize) -> Option<String> {
    std::fs::read_to_string(h.dir.join("bin").join(format!("fdtoken-{}", n - 1))).ok()
}

/// Run `n` was given none of XenoNet's shared sign-in, and no provider key.
async fn signs_in_on_its_own(h: &Host, n: usize) {
    let names = env_of(h, n).await;
    assert!(!names.iter().any(|k| k == "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR" || k == "CLAUDE_CODE_OAUTH_TOKEN"), "run {n} was given XenoNet's token: {names:?}");
    assert!(fd_token(h, n).is_none(), "run {n} read a token");
    for k in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL"] {
        assert!(!names.iter().any(|n| n == k), "run {n} kept {k}, which outranks the folder's sign-in: {names:?}");
    }
}

fn keeps_from_committing(args: &[String]) -> bool {
    args.windows(4).any(|w| w == ["--disallowedTools", "Bash(git commit:*)", "Bash(git push:*)", "Bash(gh:*)"])
}

async fn ends(h: &Host, key: &str) {
    let r = h.world.router.resume.as_ref().unwrap().runner.clone();
    let key = key.to_string();
    until("the run ends", 20, || {
        let (r, key) = (r.clone(), key.clone());
        async move { !r.running(&key) }
    })
    .await;
}

/// A ruled start in the folder: its session, once it has run.
async fn start(d: &Desk, folder: &str, n: usize) -> String {
    let (s, v) = d.host.window("POST", "/api/start", json!({ "folder": folder, "prompt": format!("tidy {n}") })).await;
    assert_eq!(s, 200, "{v}");
    let (args, _) = ran(&d.host, n).await;
    let session = format!("claude:{}", args[2]);
    ends(&d.host, &session).await;
    session
}

/// A start into a ruled project's default folder withholds the shared token,
/// and in refuse mode keeps the run from committing and pushing. Its next turns
/// (a wake, a copy) must do the same: they used to get the token back, and the
/// deny arguments were gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_ruled_starts_wake_and_copy_keep_its_placement() {
    let d = desk("accounts-wake", json!({ "claude": "nova@example.com", "mode": "refuse" }), |_, _| Vec::new()).await;
    let session = start(&d, &d.atlas, 1).await;
    let (args, _) = ran(&d.host, 1).await;
    assert!(keeps_from_committing(&args), "the start: {args:?}");
    signs_in_on_its_own(&d.host, 1).await;
    let w = &d.host.world;
    assert_eq!(accounts::of(w, &session).unwrap().map(|p| (p.root, p.ruled)), Some((None, true)), "the start recorded where it was placed");
    // A wake.
    delivery::resume_turn(w, &session, Some(&d.atlas), "wake up", false).unwrap();
    let (args, _) = ran(&d.host, 2).await;
    assert!(args.iter().any(|a| a == "--resume") && keeps_from_committing(&args), "the wake: {args:?}");
    signs_in_on_its_own(&d.host, 2).await;
    ends(&d.host, &session).await;
    // A copy, which the fake names the way Claude Code's stream does.
    delivery::resume_turn(w, &session, Some(&d.atlas), "look at this", true).unwrap();
    let (args, _) = ran(&d.host, 3).await;
    assert!(args.iter().any(|a| a == "--fork-session") && keeps_from_committing(&args), "the copy: {args:?}");
    signs_in_on_its_own(&d.host, 3).await;
    // The copy is given its id before it runs, and placed as its parent was at once:
    // nothing after the start has to name it for a turn of the copy to be held.
    let id = args.iter().position(|a| a == "--session-id").and_then(|i| args.get(i + 1)).expect("the copy's id is given to the run");
    let child = &format!("claude:{id}");
    assert_eq!(accounts::of(w, child).unwrap(), accounts::of(w, &session).unwrap(), "the copy is placed as its parent");
    xenonet_service::compose::runtime::starts::forget_forks(w);
    assert!(accounts::of(w, child).unwrap().is_some_and(|p| p.ruled && !p.deny.is_empty()), "and stays so after a restart forgets whose copy it is");
}

/// The rule may change between turns: a session placed under one account is
/// not run under a rule that now names another.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_rule_changed_since_the_start_refuses_the_next_turn() {
    let d = desk("accounts-changed", json!({ "claude": "nova@example.com" }), |_, _| Vec::new()).await;
    let session = start(&d, &d.atlas, 1).await;
    let w = &d.host.world;
    d.rules.lock().unwrap()[0].1 = json!({ "claude": "nova-work@example.com" });
    let why = delivery::resume_turn(w, &session, Some(&d.atlas), "wake up", false).unwrap_err();
    assert!(why.contains(".claude-work") && why.contains("rule"), "{why}");
    assert_eq!(fake_runs(&d.host).len(), 1, "no second run was made");
    // Put back, it runs.
    d.rules.lock().unwrap()[0].1 = json!({ "claude": "nova@example.com" });
    delivery::resume_turn(w, &session, Some(&d.atlas), "wake up", false).unwrap();
    let _ = ran(&d.host, 2).await;
}

/// A session whose start recorded where it was placed and whose record can't
/// be read is not run.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_placement_that_cannot_be_read_refuses_the_turn() {
    let d = desk("accounts-unreadable", json!({ "claude": "nova@example.com" }), |_, _| Vec::new()).await;
    let session = start(&d, &d.atlas, 1).await;
    let w = &d.host.world;
    w.store.0.db().set_setting(&format!("placed:{session}"), Some(&json!("not a placement"))).unwrap();
    let why = delivery::resume_turn(w, &session, Some(&d.atlas), "wake up", false).unwrap_err();
    assert!(why.contains("placed") || why.contains("placement"), "{why}");
    assert_eq!(fake_runs(&d.host).len(), 1);
}

/// A rule that only refuses (it names no account) still keeps the run from
/// committing and pushing.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_refuse_rule_with_no_account_still_keeps_the_run_from_committing() {
    let d = desk("accounts-refuse", json!({ "git": "nova@example.com", "mode": "refuse" }), |_, _| Vec::new()).await;
    let _ = start(&d, &d.atlas, 1).await;
    let (args, _) = ran(&d.host, 1).await;
    assert!(keeps_from_committing(&args), "{args:?}");
    // Warn mode does not.
    d.rules.lock().unwrap()[0].1 = json!({ "git": "nova@example.com", "mode": "warn" });
    let _ = start(&d, &d.atlas, 2).await;
    let (args, _) = ran(&d.host, 2).await;
    assert!(!args.iter().any(|a| a == "--disallowedTools"), "{args:?}");
}

/// A run in a named folder, or ruled, signs in with the folder's own
/// sign-in: a provider key in the service's environment is not carried over.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_run_in_the_work_folder_runs_there_without_a_provider_key() {
    let d = desk("accounts-work", json!({ "claude": "nova@example.com" }), |_, _| Vec::new()).await;
    let _ = start(&d, &d.ledger, 1).await;
    signs_in_on_its_own(&d.host, 1).await;
    // A project with no rule keeps the environment it always had.
    let other = d.atlas.replace("atlas", "scratch");
    let _ = start(&d, &other, 2).await;
    let names = env_of(&d.host, 2).await;
    assert!(names.iter().any(|k| k == "ANTHROPIC_API_KEY"), "{names:?}");
}

const WORK_SESSION: &str = "claude:0190d3e0-0000-7000-8000-0000000c0001";
const STRAY_SESSION: &str = "claude:0190d3e0-0000-7000-8000-0000000c0002";

/// A confined turn doesn't leave the default folder and the descriptor's
/// token for a folder she named, and a root that isn't one she named is no
/// folder to run in.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_confined_turn_stays_in_the_default_folder_and_a_root_must_be_one_she_named() {
    let d = desk("accounts-confined", json!({ "claude": "nova@example.com" }), |_, work| {
        let mut a = sess("claude:0190d3e0-0000-7000-8000-0000000c0001", "Work", "/x");
        a.root = Some(work.to_string());
        let mut b = sess("claude:0190d3e0-0000-7000-8000-0000000c0002", "Stray", "/x");
        b.root = Some("/somewhere/else".into());
        vec![a, b]
    })
    .await;
    let w = &d.host.world;
    let scratch = d.atlas.replace("atlas", "scratch");
    // Control: unconfined, the session of the work folder runs in it.
    delivery::resume_turn(w, WORK_SESSION, Some(&scratch), "wake up", false).unwrap();
    let _ = ran(&d.host, 1).await;
    let names = env_of(&d.host, 1).await;
    assert!(names.iter().any(|k| k == "CLAUDE_CONFIG_DIR"), "{names:?}");
    ends(&d.host, WORK_SESSION).await;
    // Confined, it is refused.
    sandbox::keep(w, WORK_SESSION, &sandbox::Record { folder: scratch.clone(), domains: Vec::new(), for_others: None }).unwrap();
    let why = delivery::resume_turn(w, WORK_SESSION, Some(&scratch), "wake up", false).unwrap_err();
    assert!(why.contains("sandbox") && why.contains("folder"), "{why}");
    // A root nobody named is no root.
    let why = delivery::resume_turn(w, STRAY_SESSION, Some(&scratch), "wake up", false).unwrap_err();
    assert!(why.contains("/somewhere/else"), "{why}");
    assert_eq!(fake_runs(&d.host).len(), 1);
}

/// The wake budget reads the plan of the folder a session runs in, not the
/// default folder's.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_wake_budget_reads_the_plan_of_the_sessions_own_folder() {
    use xenonet_engine::delivery::router::Host as _;
    let d = desk("accounts-usage", json!({ "claude": "nova@example.com" }), |_, work| {
        let mut a = sess("claude:0190d3e0-0000-7000-8000-0000000c0001", "Work", "/x");
        a.root = Some(work.to_string());
        vec![a, sess("claude:0190d3e0-0000-7000-8000-0000000c0003", "Home", "/x")]
    })
    .await;
    let mut host = delivery::RouterHost::of(&d.host.world);
    assert_eq!(host.usage_of(WORK_SESSION), Some(0.92), "the work folder's plan");
    assert_eq!(host.usage_of("claude:0190d3e0-0000-7000-8000-0000000c0003"), Some(0.4), "the default folder's");
}
