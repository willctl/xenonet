//! `xenolith curator`: files queued notes into the wiki, and answers the window's questions (Ask).
//! It runs as the user (the tray hosts it), because it uses the user's ChatGPT
//! sign-in through the Codex CLI.
//!
//!   xenolith curator [--parent-stdin]   run until stopped; --parent-stdin: stop when stdin closes
//!   xenolith curator --once             file everything that is due now, ignoring the debounce, then exit
//!   xenolith curator --retry-dead       give failed (dead-lettered) notes another round of attempts
//!   xenolith curator --file-raw-dead    file failed notes into the log as they are, without the model
//!   xenolith curator --login-status     print {"signedIn":...} for the curator's Codex login
//!   xenolith curator --asks-only        answer the window's questions without filing notes
//!   --space <id>                         with --retry-dead, --file-raw-dead or --lint: that space's
//!                                        notes and pages instead of this machine's (machine:local)
//!
//! Each space has its own curator (xenolith_core::spaces): this process runs one for this machine's
//! space and one for every other space, and starts one for a space created while it runs. A space
//! this machine only follows has its curator on another machine (xenolith_core::sync): this process
//! syncs it once a minute instead.

use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use xenolith_core::askworker::{AskCfg, AskWorker};
use xenolith_core::codex;
use xenolith_core::curator::{self, Curator, CuratorCfg, log};
use xenolith_core::inbox;
use xenolith_core::paths::{AppPaths, process_env};
use xenolith_core::reqlog::RequestLog;
use xenolith_core::spaces::{self, SpaceId};
use xenolith_core::waker::Waker;
use xenolith_core::wiki;

/// The spaces this process curates besides machine:local, each with its curator.
type Others = Arc<Mutex<Vec<(SpaceId, Arc<Curator>, std::thread::JoinHandle<()>)>>>;

/// Starts a curator for every space that has none yet; returns how many it started.
fn start_space_curators(root: &Path, cfg: &CuratorCfg, reqlog: &Arc<RequestLog>, others: &Others) -> usize {
    let mut have = others.lock().unwrap();
    let mut started = 0;
    let followed = xenolith_core::sync::Config::load(&AppPaths::current());
    for id in spaces::list(root).into_iter().filter(|s| !s.is_local() && followed.curator_of(s).is_none()) {
        if have.iter().any(|(s, _, _)| *s == id) {
            continue;
        }
        let dir = spaces::space_dir(root, &id);
        let c = Arc::new(Curator::new(&dir, cfg.clone(), reqlog.clone(), false, Waker::new()));
        let (runner, label) = (c.clone(), id.to_string());
        let t = std::thread::spawn(move || {
            if let Err(e) = runner.run() {
                log(&format!("the curator of {label} stopped: {e:?}"));
            }
        });
        log(&format!("curating {id} too ({})", wiki::disp(&dir)));
        have.push((id, c, t));
        started += 1;
    }
    started
}

fn fatal(msg: &str) -> ! {
    log(&format!("fatal: {msg}"));
    std::process::exit(1)
}

pub fn run(args: &[String]) -> ! {
    let has = |f: &str| args.iter().any(|a| a == f);
    let env = process_env();
    let config = wiki::read_config(&env).unwrap_or_else(|e| fatal(&e.to_string()));
    xenolith_core::embed::configure(&config);
    let paths = AppPaths::current();
    let cfg = CuratorCfg::from_config(&config, &paths);
    let (root, _) = wiki::resolve_wiki_dir(&env).unwrap_or_else(|e| fatal(&e.to_string()));
    wiki::ensure_wiki(&root).unwrap_or_else(|e| fatal(&e.to_string()));
    // The maintenance commands work on one space: this machine's unless --space names another.
    let wiki_dir: PathBuf = match args.iter().position(|a| a == "--space").and_then(|i| args.get(i + 1)) {
        Some(s) => {
            let id = SpaceId::parse(s).unwrap_or_else(|e| fatal(&e.to_string()));
            if !spaces::exists(&root, &id) {
                fatal(&format!("no space {id}"));
            }
            spaces::space_dir(&root, &id)
        }
        None => root.clone(),
    };
    let days = config["logs"]["retentionDays"].as_i64().filter(|d| *d > 0).unwrap_or(30);
    let reqlog = Arc::new(RequestLog::new(&paths.log_dir, "curator", days));

    if has("--login-status") {
        let (signed_in, detail) = codex::login_status(&cfg.model_cfg());
        println!("{}", json!({ "signedIn": signed_in, "detail": detail }));
        std::process::exit(0);
    }
    if has("--retry-dead") {
        let n = inbox::retry_notes(&wiki_dir, true).unwrap_or_else(|e| fatal(&e.to_string()));
        reqlog.write(vec![("kind", json!("curator")), ("event", json!("retry-dead")), ("notes", json!(n))]);
        println!("{}", json!({ "retried": n }));
        std::process::exit(0);
    }
    if has("--file-raw-dead") {
        let dead: Vec<String> = inbox::list_notes(&wiki_dir).unwrap_or_else(|e| fatal(&e.to_string())).into_iter().filter(|n| n.status == "dead").map(|n| n.id).collect();
        for id in &dead {
            if let Err(e) = curator::file_raw(&wiki_dir, id) {
                fatal(&format!("{e:?}"));
            }
        }
        reqlog.write(vec![("kind", json!("curator")), ("event", json!("file-raw")), ("notes", json!(dead))]);
        println!("{}", json!({ "filed": dead.len() }));
        std::process::exit(0);
    }

    if has("--lint") {
        // Review the named pages (or every page that is due) now, then exit.
        let named: Vec<String> = args.iter().skip_while(|a| *a != "--lint").skip(1).take_while(|a| !a.starts_with("--")).cloned().collect();
        let c = Curator::new(&wiki_dir, cfg.clone(), reqlog.clone(), true, Waker::new());
        let mut proposed = 0;
        let mut reviewed = vec![];
        if named.is_empty() {
            while let Some(slug) = xenolith_core::lint::next_due(&wiki_dir, &cfg) {
                proposed += c.lint_one(&slug);
                reviewed.push(slug);
            }
        } else {
            for slug in &named {
                proposed += c.lint_one(slug);
                reviewed.push(slug.clone());
            }
        }
        println!("{}", json!({ "reviewed": reviewed, "proposed": proposed }));
        std::process::exit(0);
    }
    let once = has("--once");
    let asks_only = has("--asks-only");
    if once && !asks_only {
        // File what is due in the other spaces first; this machine's space follows below.
        let followed = xenolith_core::sync::Config::load(&paths);
        for id in spaces::list(&root).into_iter().filter(|s| !s.is_local() && followed.curator_of(s).is_none()) {
            let c = Arc::new(Curator::new(&spaces::space_dir(&root, &id), cfg.clone(), reqlog.clone(), true, Waker::new()));
            if let Err(e) = c.run() {
                fatal(&format!("{id}: {e:?}"));
            }
        }
    }
    let others: Others = Arc::new(Mutex::new(vec![]));
    let curator = (!asks_only).then(|| Arc::new(Curator::new(&wiki_dir, cfg.clone(), reqlog.clone(), once, Waker::new())));
    // The same process answers the window's questions (Ask): it is the one that runs as the user.
    let asker = (!once).then(|| {
        let server = std::env::current_exe().unwrap_or_else(|_| "xenolith".into());
        Arc::new(AskWorker::new(&wiki_dir, AskCfg::from_config(&config, &cfg), reqlog.clone(), server, paths.clone(), Waker::new()))
    });
    let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = {
        let (curator, asker, others, stopping) = (curator.clone(), asker.clone(), others.clone(), stopping.clone());
        move || {
            stopping.store(true, Ordering::SeqCst);
            for c in curator.iter().chain(others.lock().unwrap().iter().map(|(_, c, _)| c)) {
                c.abort.store(true, Ordering::SeqCst);
                c.waker.stop();
            }
            if let Some(a) = &asker {
                a.stop();
            }
        }
    };
    if has("--parent-stdin") {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut sink = [0u8; 4096];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
            stop();
        });
    }
    signals::on_stop(stop);
    let chosen = cfg.with_choice(&wiki_dir);
    log(&format!(
        "v{} started: wiki {}, model {} ({}; the window can change it), codex {}, CODEX_HOME {}{}",
        xenolith_core::VERSION,
        wiki::disp(&wiki_dir),
        chosen.model,
        chosen.reasoning_effort,
        cfg.codex_path,
        wiki::disp(&cfg.codex_home),
        if curator.is_some() { "" } else { ", answering questions only" }
    ));
    reqlog.write(vec![
        ("kind", json!("curator")),
        ("event", json!("start")),
        ("model", json!(chosen.model)),
        ("once", if once { json!(true) } else { Value::Null }),
        ("asksOnly", if curator.is_none() { json!(true) } else { Value::Null }),
    ]);
    let ask_thread = asker.clone().map(|a| std::thread::spawn(move || a.run()));
    // The other spaces' curators, and a look for new spaces every few seconds.
    let watcher = (curator.is_some() && !once).then(|| {
        let (root, cfg, reqlog, others, stopping) = (root.clone(), cfg.clone(), reqlog.clone(), others.clone(), stopping.clone());
        std::thread::spawn(move || {
            let mut synced_at: Option<std::time::Instant> = None;
            while !stopping.load(Ordering::SeqCst) {
                start_space_curators(&root, &cfg, &reqlog, &others);
                // The spaces this machine follows: push their notes and pull their pages each minute.
                if synced_at.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(60)) {
                    synced_at = Some(std::time::Instant::now());
                    for r in xenolith_core::sync::sync_all(&AppPaths::current(), &root) {
                        if let Some(e) = r["error"].as_str() {
                            log(&format!("sync of {}: {e}", r["space"].as_str().unwrap_or("?")));
                        }
                    }
                }
                for _ in 0..50 {
                    if stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        })
    });
    let mut failed = None;
    if let Some(Err(e)) = curator.as_ref().map(|c| c.run()) {
        failed = Some(format!("{e:?}"));
        if let Some(a) = &asker {
            a.stop();
        }
    }
    stop_others(&stopping, &others, watcher);
    if let Some(t) = ask_thread {
        let _ = t.join();
    }
    reqlog.write(vec![("kind", json!("curator")), ("event", json!("stop"))]);
    if let Some(e) = failed {
        fatal(&e);
    }
    log("stopped");
    std::process::exit(0)
}

/// Stops the other spaces' curators (when this machine's stopped first, on an error) and waits for them.
fn stop_others(stopping: &std::sync::atomic::AtomicBool, others: &Others, watcher: Option<std::thread::JoinHandle<()>>) {
    stopping.store(true, Ordering::SeqCst);
    if let Some(w) = watcher {
        let _ = w.join();
    }
    let all: Vec<_> = others.lock().unwrap().drain(..).collect();
    for (_, c, _) in &all {
        c.abort.store(true, Ordering::SeqCst);
        c.waker.stop();
    }
    for (_, _, t) in all {
        let _ = t.join();
    }
}

/// SIGINT and SIGTERM stop gracefully (heartbeat "stopped", locks released).
mod signals {
    #[cfg(unix)]
    pub fn on_stop(stop: impl Fn() + Send + 'static) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static GOT: AtomicBool = AtomicBool::new(false);
        extern "C" fn handler(_: libc::c_int) {
            GOT.store(true, Ordering::SeqCst);
        }
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
        unsafe {
            libc::signal(libc::SIGINT, handler as *const () as libc::sighandler_t);
            libc::signal(libc::SIGTERM, handler as *const () as libc::sighandler_t);
        }
        std::thread::spawn(move || {
            while !GOT.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            stop();
        });
    }

    #[cfg(not(unix))]
    pub fn on_stop(_stop: impl Fn() + Send + 'static) {
        // Windows: the tray stops the curator by closing its stdin (--parent-stdin).
    }
}
