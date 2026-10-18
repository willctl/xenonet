//! core/state.js's runtime: XenoNetState, what the service knows,
//! refreshed. A refresh gathers each app's sessions, scans the projects
//! folder, reads the wiki, builds the map and links identities; between
//! refreshes the live tailer follows the transcripts. What a refresh
//! decides, and what it remembers from one to the next, are
//! xenonet-state's state.rs (`Memory`); the gathering, the timers, the
//! map's index and the events (pulse, activity, refusal, tokens, graph,
//! live) are here.

use crate::blocks::{Blocks, Reported};
use crate::codex::CodexClient;
use crate::config::MapConfig;
use crate::demo::DemoWorld;
use crate::gatherer::{Gather, Options};
use crate::git::RepoWatch;
use crate::identities::IdentityWatch;
use crate::live::{Event, Live};
use crate::projects::Projects;
use crate::search::TranscriptSearch;
use crate::stats::Stats;
use crate::store::StateStore;
use crate::util::{get, items, lock, log, now_ms, num, obj, s};
use crate::vendor::Vendor;
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::watch;
use xenonet_base::paths::path_key;
use xenonet_base::text::{js_string, truthy};
use xenonet_service::cores::{CoreError, Runs, State, R};
use xenonet_service::glue::From;
use xenonet_service::wire::Emitter;
use xenonet_sources::reported_runs::ReportedRuns;
use xenonet_state::state::{self as decide, Memory};
use xenonet_state::{classify, digest, redact, rename, workspace};

// An event's fields, as the telemetry takes them (undefined as None).
type Fields = Vec<(String, Option<Value>)>;

const LIVE_THROTTLE_MS: u64 = 500;
const REPO_POLL_MS: u64 = 20_000;
const HEARTBEAT_MS: u64 = 2000;
const NEW_FILE_MS: u64 = 1200;
const DEMO_STEP_MS: u64 = 2200;
// The wiki's folder, looked at this often for a change (the JS watches it),
// and the map made again this long after one.
const WIKI_LOOK_MS: u64 = 1000;
const WIKI_SETTLE_MS: u64 = 800;

/// What the map is read from: the demo's made-up world, or this machine's
/// apps.
pub enum Readers {
    Demo(Arc<DemoWorld>),
    Real { claude: Arc<crate::claude::Claude> },
}

/// The map as the last refresh made it, and its index.
#[derive(Default)]
struct Map_ {
    graph: Option<Value>,
    index: HashMap<String, usize>,
    parent: HashMap<String, String>,
    sources: Value,
    projects: Vec<Value>,
    scan: Option<Value>,
    session_dirs: Vec<Value>,
}

struct Inner {
    map: Map_,
    memory: Memory,
    /// The projects folder in use (workspace.js's applyRoot).
    projects_dir: String,
    /// The names you gave the demo's sessions, which exist only in XenoNet.
    demo_titles: Vec<(String, Value)>,
    /// Just after the projects folder changed: its key, for a refresh that
    /// reads the repos' status first.
    repo_pass_for: Option<String>,
}

type Done = Option<Result<Value, String>>;

pub struct MapState {
    me: Weak<MapState>,
    pub cfg: MapConfig,
    store: Arc<StateStore>,
    vendor: Arc<Vendor>,
    emitter: Emitter,
    readers: Readers,
    pub live: Arc<Live>,
    codex: Arc<CodexClient>,
    projects_scan: Arc<Projects>,
    repos: Arc<RepoWatch>,
    identities: Option<Arc<IdentityWatch>>,
    search: Arc<TranscriptSearch>,
    stats: Arc<Stats>,
    pub blocks: Blocks,
    claude_sessions: Arc<Gather>,
    codex_threads: Arc<Gather>,
    codex_projects: Arc<Gather>,
    pub runs: Mutex<ReportedRuns>,
    /// The native readers and numbat's, when they're on.
    observed: Vec<Arc<crate::observed::Observed>>,
    inner: Mutex<Inner>,
    refreshing: Mutex<Option<watch::Receiver<Done>>>,
    repo_pass: tokio::sync::Mutex<()>,
    version: AtomicU64,
    listening: AtomicBool,
    live_queued: AtomicBool,
    new_files: AtomicU64,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// The events it sent, for the tests to read.
    #[cfg(test)]
    pub sent: Mutex<Vec<(String, Value)>>,
}

fn err(m: impl Into<String>) -> CoreError {
    CoreError::new(m)
}

fn status(code: u16, m: impl Into<String>) -> CoreError {
    CoreError::status(code, m)
}

/// A gatherer's run as a source of the map: what it found, and how it went.
fn gathered(name: &str, g: &Gather, value: Value) -> (Value, Value) {
    let last = g.last();
    let ok = last.as_ref().is_some_and(|l| l.ok);
    let ms = last.as_ref().map(|l| num(l.ms));
    let error = last.and_then(|l| l.error).map(Value::String);
    let value = if ok { value } else { json!([]) };
    let st = decide::source_status(name, ok, ms.as_ref(), ok.then_some(&value), error.as_ref());
    (value, st)
}

impl MapState {
    #[expect(clippy::too_many_arguments, reason = "the parts a state is made of, each made by wire")]
    pub fn new(
        cfg: MapConfig,
        store: Arc<StateStore>,
        vendor: Arc<Vendor>,
        emitter: Emitter,
        readers: Readers,
        live: Arc<Live>,
        codex: Arc<CodexClient>,
        version: &str,
    ) -> Arc<MapState> {
        let codex_roots =
            [std::path::Path::new(&cfg.codex_home).join("sessions"), std::path::Path::new(&cfg.codex_home).join("archived_sessions")].map(|p| p.to_string_lossy().into_owned());
        let search = TranscriptSearch::new(&std::path::Path::new(&cfg.cache_dir).join("search.sqlite").to_string_lossy(), &cfg.claude_projects_dir, &codex_roots, vendor.clone());
        let stats = Stats::new(&cfg.claude_projects_dir, &codex_roots, vendor.clone());
        let identities = cfg.identities.then(|| {
            IdentityWatch::new(
                crate::identities::Files {
                    home: cfg.home_dir.clone(),
                    claude_config_file: cfg.claude_config_file.clone(),
                    codex_home: cfg.codex_home.clone(),
                    gh_config_dir: cfg.gh_config_dir.clone(),
                    claude_projects_dir: cfg.claude_projects_dir.clone(),
                },
                vendor.clone(),
                &cfg.env,
            )
        });
        let gatherer = |name: &str| Arc::new(Gather::new(name, Options { fallback: json!([]), ..Default::default() }));
        let (claude_sessions, codex_threads, codex_projects) = (gatherer("claude-sessions"), gatherer("codex-threads"), gatherer("codex-projects"));
        // The projects folder you chose, if you did (workspace.js), as
        // state.json has it: the store itself is read at the first refresh,
        // with the rest of what it saves then.
        let saved = workspace::saved_root(&std::path::Path::new(&cfg.state_dir).join("state.json").to_string_lossy()).map(Value::String);
        let root = workspace::root(cfg.env_projects_dir.as_deref(), saved.as_ref(), &cfg.home_dir);
        let projects_dir = root["path"].as_str().unwrap_or(&cfg.projects_dir).to_string();
        let _ = version;
        let state = Arc::new_cyclic(|me| MapState {
            me: me.clone(),
            store: store.clone(),
            vendor: vendor.clone(),
            emitter,
            readers,
            live: live.clone(),
            codex: codex.clone(),
            projects_scan: Arc::new(Projects::default()),
            repos: Arc::new(RepoWatch::default()),
            identities,
            search,
            stats,
            blocks: Blocks::default(),
            claude_sessions,
            codex_threads,
            codex_projects,
            runs: Mutex::new(ReportedRuns::new()),
            observed: crate::observed::Observed::all(&cfg, &vendor),
            inner: Mutex::new(Inner { map: Map_ { sources: json!([]), ..Default::default() }, memory: Memory::new(), projects_dir, demo_titles: Vec::new(), repo_pass_for: None }),
            refreshing: Mutex::new(None),
            repo_pass: tokio::sync::Mutex::new(()),
            version: AtomicU64::new(0),
            listening: AtomicBool::new(false),
            live_queued: AtomicBool::new(false),
            new_files: AtomicU64::new(0),
            tasks: Mutex::new(Vec::new()),
            #[cfg(test)]
            sent: Mutex::new(Vec::new()),
            cfg,
        });
        // The building blocks it's made of, for /api/health and the doctor.
        // The ones that read for a source of the map say which.
        let b = &state.blocks;
        let _ = b.add(store, None);
        let _ = b.add(state.claude_sessions.clone(), Some("claude"));
        let _ = b.add(state.codex_threads.clone(), Some("codex"));
        let _ = b.add(state.codex_projects.clone(), Some("codex-projects"));
        let _ = b.add(codex, Some("codex"));
        let _ = b.add(live.block(), Some("live"));
        for l in vendor.blocks() {
            let _ = b.add(l, None);
        }
        if let Some(a) = live.anthropic.clone() {
            let _ = b.add(a, None);
        }
        if let Some((d, _)) = live.desktop.clone() {
            let _ = b.add(d, None);
        }
        if let Some(i) = state.identities.clone() {
            let _ = b.add(i, Some("identities"));
        }
        for o in &state.observed {
            let _ = b.add(o.clone(), Some(o.source));
        }
        let _ = b.add(state.repos.clone(), None);
        let _ = b.add(state.projects_scan.clone(), Some("projects"));
        state
    }

    fn arc(&self) -> Option<Arc<MapState>> {
        self.me.upgrade()
    }

    fn emit(&self, name: &str, data: Value) {
        #[cfg(test)]
        lock(&self.sent).push((name.to_string(), data.clone()));
        self.emitter.emit(From::State, name, data);
    }

    fn demo(&self) -> Option<&Arc<DemoWorld>> {
        match &self.readers {
            Readers::Demo(d) => Some(d),
            Readers::Real { .. } => None,
        }
    }

    pub fn projects_dir(&self) -> String {
        lock(&self.inner).projects_dir.clone()
    }

    fn node_of(&self, id: &str) -> Option<Value> {
        let g = lock(&self.inner);
        let i = *g.map.index.get(id)?;
        g.map.graph.as_ref().and_then(|gr| gr.get("nodes")).and_then(|n| n.get(i)).cloned()
    }

    fn has(&self, id: &str) -> bool {
        lock(&self.inner).map.index.contains_key(id)
    }

    fn all_nodes(&self) -> Vec<Value> {
        let g = lock(&self.inner);
        g.map.graph.as_ref().map(|gr| items(gr.get("nodes")).to_vec()).unwrap_or_default()
    }

    fn title(&self, id: &Value) -> Value {
        let n = id.as_str().and_then(|i| self.node_of(i));
        decide::title_of(n.as_ref().and_then(|n| n.get("title")), n.as_ref().and_then(|n| n.get("name")), id)
    }

    fn home(&self, id: &str) -> Option<String> {
        if id.is_empty() {
            return None;
        }
        let g = lock(&self.inner);
        let nodes = g.map.graph.as_ref().map(|gr| items(gr.get("nodes"))).unwrap_or(&[]);
        let mut kind_of = |cur: &str| g.map.index.get(cur).and_then(|&i| nodes.get(i)).map(|n| n.get("kind").map_or_else(|| "undefined".into(), js_string));
        let mut parent_of = |cur: &str| g.map.parent.get(cur).cloned();
        decide::home_of(id, &mut kind_of, &mut parent_of)
    }

    // ---------- refreshing ----------

    /// One refresh at a time: one asked for while another runs shares it.
    pub async fn refresh_map(&self) -> Result<Value, String> {
        let Some(me) = self.arc() else { return Err("XenoNet is stopping".into()) };
        let mut rx = {
            let mut g = lock(&self.refreshing);
            match g.as_ref() {
                Some(rx) => rx.clone(),
                None => {
                    let (tx, rx) = watch::channel(None);
                    *g = Some(rx.clone());
                    tokio::spawn(async move {
                        let r = me.refresh_now().await;
                        *lock(&me.refreshing) = None;
                        let _ = tx.send(Some(r));
                    });
                    rx
                }
            }
        };
        loop {
            if let Some(r) = rx.borrow().clone() {
                return r;
            }
            if rx.changed().await.is_err() {
                return Err("the refresh stopped".into());
            }
        }
    }

    /// A refresh that sees a change just made: one already running read the
    /// state from before it.
    pub async fn refresh_after(&self) -> Result<Value, String> {
        let running = lock(&self.refreshing).clone();
        if let Some(mut rx) = running {
            while rx.borrow().is_none() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
        self.refresh_map().await
    }

    async fn gather_sessions(&self, app: &str) -> Result<Value, String> {
        match &self.readers {
            Readers::Demo(d) => d.sessions(app).map(Value::Array),
            Readers::Real { claude } => {
                if app == "codex" {
                    self.codex.threads().await.map(Value::Array)
                } else {
                    claude.sessions().await.map(Value::Array)
                }
            }
        }
    }

    async fn gather_native(&self) -> Result<Value, String> {
        match &self.readers {
            Readers::Demo(d) => Ok(d.native_projects()),
            Readers::Real { .. } => {
                let home = self.cfg.codex_home.clone();
                let vendor = self.vendor.clone();
                Ok(Value::Array(crate::util::blocking(move || crate::codex::native_projects(&home, &vendor)).await))
            }
        }
    }

    // The rules a scan of `root` follows: Codex's picks for that folder,
    // then yours on top.
    fn project_rules(&self, codex: bool, root: &str) -> Value {
        let picks = if codex { self.store.codex_picks(path_key(root).as_deref()) } else { None };
        decide::project_rules(codex, root, picks.as_ref(), &self.store.project_rules())
    }

    async fn refresh_now(&self) -> Result<Value, String> {
        // The projects folder as this refresh found it, for all of it.
        let root = self.projects_dir();
        let (codex, native, claude) = tokio::join!(
            self.codex_threads.run(false, None, || self.gather_sessions("codex")),
            self.codex_projects.run(false, None, || self.gather_native()),
            self.claude_sessions.run(false, None, || self.gather_sessions("claude")),
        );
        let (codex, codex_st) = gathered("codex", &self.codex_threads, codex);
        let (native, native_st) = gathered("codex-projects", &self.codex_projects, native);
        let (claude, claude_st) = gathered("claude", &self.claude_sessions, claude);
        // Keep the previous sessions for a source that failed this round, so
        // a transient error doesn't make half the graph vanish.
        let prev: Vec<Value> = self.all_nodes().into_iter().filter(|n| s(n, "kind") == Some("session")).collect();
        // Observed runs, unless XenoNet reads the same run itself.
        let readers = self.observed.clone();
        crate::util::blocking(move || readers.iter().for_each(|o| o.read())).await;
        let observed = self.observed_sessions();
        let titles: HashMap<String, Value> = lock(&self.inner).demo_titles.iter().cloned().collect();
        let ok = |st: &Value| st.get("ok") == Some(&Value::Bool(true));
        let sessions = decide::merge_sessions(&prev, (ok(&codex_st), items(Some(&codex))), (ok(&claude_st), items(Some(&claude))), &observed, &titles);
        // Where sessions ran is evidence of where the projects are, so the
        // scan comes after the session lists.
        let session_dirs = decide::session_dirs(&sessions);
        lock(&self.inner).map.session_dirs = session_dirs.clone();
        let rules = self.project_rules(true, &root);
        let t0 = now_ms();
        let scanned = self.projects_scan.scan(&root, session_dirs, rules.clone(), self.cfg.project_limits.clone(), false, None).await;
        let (mut projects, scan, mut projects_st) = match scanned {
            Ok(found) => {
                let list = Value::Array(found.projects.clone());
                (found.projects, Some(found.scan), decide::source_status("projects", true, Some(&num(now_ms() - t0)), Some(&list), None))
            }
            Err(e) => (Vec::new(), None, decide::source_status("projects", false, Some(&num(now_ms() - t0)), None, Some(&json!(e)))),
        };
        for p in projects.iter_mut() {
            let key = p.get("key").map(js_string).unwrap_or_default();
            if let Some(m) = p.as_object_mut() {
                m.insert("rule".into(), rules.get(&key).cloned().unwrap_or(Value::Null));
            }
        }
        if let Some(sc) = scan.as_ref().filter(|sc| truthy(sc.get("ok"))) {
            projects_st["count"] = sc.get("projects").cloned().unwrap_or(Value::Null);
            projects_st["note"] = json!(decide::projects_note(sc));
        }
        let repo_pass = {
            let mut g = lock(&self.inner);
            g.map.projects = projects.clone();
            g.map.scan = scan.clone();
            let key = path_key(&root);
            let now = g.repo_pass_for.is_some() && g.repo_pass_for == key;
            if now {
                g.repo_pass_for = None;
            }
            now
        };
        // Just after the projects folder changed, its repos' status and git
        // identities are read first, so the new map goes out whole.
        if repo_pass {
            self.repo_pass().await;
        }
        let wiki = match &self.cfg.wiki_dir {
            Some(dir) => {
                let (dir, list, rootv, server) = (dir.clone(), Value::Array(projects.clone()), json!(root), self.cfg.wiki_server.clone());
                crate::util::blocking(move || {
                    xenonet_sources::wiki::read_wiki(&dir, Some(&list), Some(&rootv), now_ms(), xenonet_sources::wiki::Out::Json(server))
                        .ok()
                        .and_then(|o| crate::util::wiki_out(&o))
                })
                .await
                .filter(|w| !w.is_null())
            }
            None => None,
        };
        let hidden = self.cfg.hidden();
        let mut resources: Vec<Value> = items(self.live.snapshot().get("resources")).to_vec();
        if self.cfg.demo {
            resources.push(json!({ "server": "reference-library", "ts": null }));
        }
        resources.retain(|r| r.get("server").map(js_string).is_some_and(|srv| decide::visible_resource(&srv, &hidden)));
        let input = json!({
            "projectsDir": root,
            "homeDir": self.cfg.home_dir,
            "projects": projects,
            "sessions": sessions,
            "nativeProjects": native,
            "resources": resources,
            "wiki": wiki,
            "repos": self.repos.status().into_iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
            "layout": self.store.layout(),
            "now": num(now_ms()),
            "windows": self.cfg.windows,
        });
        let parent_of = xenonet_state::projects::relations(&projects).map_err(|e| e.0)?.0;
        let mut graph = xenonet_state::graph::build_graph(&input, &parent_of.into_iter().collect());
        if let Some(Value::Array(nodes)) = graph.get_mut("nodes") {
            for n in nodes.iter_mut() {
                let acts = xenonet_app::open::actions_for(Some(n));
                if let Some(m) = n.as_object_mut() {
                    m.insert("actions".into(), Value::Array(acts));
                }
            }
        }
        // Who each session ran as, and whose each project is.
        let identities_st = match &self.identities {
            None => None,
            Some(w) => {
                let t0 = now_ms();
                let linked = w.link(&mut graph, &self.store);
                if linked.is_ok() {
                    self.note_signins();
                }
                Some(match linked {
                    Ok(list) => {
                        let v = Value::Array(list.clone());
                        let mut st = decide::source_status("identities", true, Some(&num(now_ms() - t0)), Some(&v), None);
                        st["note"] = json!(decide::identities_note(&list));
                        st
                    }
                    Err(e) => decide::source_status("identities", false, Some(&num(now_ms() - t0)), None, Some(&json!(e))),
                })
            }
        };
        if let Some(g) = graph.as_object_mut() {
            g.entry("identities").or_insert_with(|| json!([]));
            g.insert("hideIdentities".into(), Value::Bool(self.cfg.hide_identities));
            // The demo's sessions exist only in XenoNet.
            g.insert("demo".into(), Value::Bool(self.cfg.demo));
        }
        let live = lock(&self.live.status).clone();
        let search = self.search.status_json();
        let mut sources = vec![projects_st, codex_st, native_st, claude_st];
        sources.extend(self.observed.iter().map(|o| o.status()));
        sources.push(json!({ "name": "live", "ok": live.watching, "count": live.files }));
        let mut wiki_st = json!({ "name": "wiki", "ok": wiki.is_some(), "count": wiki.as_ref().map_or(0, |w| items(w.get("pages")).len()) });
        if wiki.is_none() {
            wiki_st["error"] = json!("Agent Wiki folder not found");
        }
        sources.push(wiki_st);
        sources.push(json!({ "name": "search", "ok": !truthy(search.get("errors")), "count": search.get("docs").cloned().unwrap_or(Value::Null), "ms": search.get("ms").cloned().unwrap_or(Value::Null) }));
        if let Some(st) = identities_st {
            sources.push(st);
        }
        let sources = Value::Array(sources);
        graph["sources"] = sources.clone();
        let (pulses, changed) = {
            let mut g = lock(&self.inner);
            let pulses = g.memory.updated(&sessions);
            let changed = g.memory.changed(&graph);
            let nodes = items(graph.get("nodes"));
            g.map.index = nodes.iter().enumerate().map(|(i, n)| (n.get("id").map(js_string).unwrap_or_default(), i)).collect();
            g.map.parent = items(graph.get("links"))
                .iter()
                .filter(|l| s(l, "kind") != Some("lineage-cross"))
                .map(|l| (l.get("target").map(js_string).unwrap_or_default(), l.get("source").map(js_string).unwrap_or_default()))
                .collect();
            g.map.sources = sources;
            g.map.graph = Some(graph.clone());
            (pulses, changed)
        };
        self.version.fetch_add(1, Ordering::SeqCst);
        if changed {
            self.emit("graph", graph.clone());
        }
        for id in pulses {
            self.emit("pulse", json!({ "nodeId": id, "reason": "updated" }));
        }
        Ok(graph)
    }

    /// The runs XenoNet observes without a reader of its own: the runs
    /// actors report (the native readers and numbat are the JS service's
    /// for now).
    fn observed_sessions(&self) -> Vec<Value> {
        let mut all: Vec<Value> = self.observed.iter().flat_map(|o| o.sessions()).collect();
        all.extend(lock(&self.runs).sessions(now_ms()).iter().filter_map(crate::util::native_out));
        // One session per id: the first source to list it wins.
        let ids: Vec<Option<Value>> = all.iter().map(|x| x.get("id").cloned()).collect();
        let first = xenonet_sources::reported_runs::firsts(&ids);
        all.into_iter().zip(first).filter(|(_, f)| *f).map(|(x, _)| x).collect()
    }

    // A sign-in that changed since the last refresh goes to the activity
    // feed.
    fn note_signins(&self) {
        let Some(w) = &self.identities else { return };
        let accounts = w.accounts();
        let events = lock(&self.inner).memory.note_signins(&accounts, now_ms());
        for ev in events {
            self.emit("activity", ev);
        }
    }

    // The git status loop's pass, with each repo's git identity read on the
    // same beat, over the projects as they are when it starts. One at a
    // time. Says whether anything changed.
    async fn repo_pass(&self) -> bool {
        let _one = self.repo_pass.lock().await;
        let projects = lock(&self.inner).map.projects.clone();
        let sig = |r: &RepoWatch| {
            xenonet_base::json::stringify(&Value::Array(r.status().into_iter().map(|(k, v)| json!([k, v.get("dirty"), v.get("branch"), v.get("ahead")])).collect()), 0)
        };
        let before = sig(&self.repos);
        self.repos.refresh(&projects).await;
        let ids = match &self.identities {
            Some(w) => w.refresh_repos(&projects).await,
            None => false,
        };
        before != sig(&self.repos) || ids
    }

    async fn refresh_repos(&self) {
        if self.repo_pass().await {
            let _ = self.refresh_map().await;
        } else {
            let running = lock(&self.refreshing).clone();
            if let Some(mut rx) = running {
                while rx.borrow().is_none() {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    // ---------- live ----------

    /// The tailer's snapshot as the page gets it.
    pub fn snapshot(&self) -> Value {
        let snap = self.live.snapshot();
        let acks = self.store.acks();
        let signed_in = lock(&self.inner).memory.signed_in();
        let hidden = self.cfg.hidden();
        let g = lock(&self.inner);
        let nodes = g.map.graph.as_ref().map(|gr| items(gr.get("nodes"))).unwrap_or(&[]);
        let mut node = |id: &str| g.map.index.get(id).and_then(|&i| nodes.get(i)).cloned();
        decide::live_snapshot(snap, &hidden, &acks, signed_in.as_ref(), &mut node)
    }

    // Snapshots go out at most every LIVE_THROTTLE_MS.
    fn queue_live(&self) {
        if self.live_queued.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(me) = self.arc() else { return };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(LIVE_THROTTLE_MS)).await;
            me.live_queued.store(false, Ordering::SeqCst);
            let snap = me.snapshot();
            me.emit("live", snap);
        });
    }

    // When an agent ends a turn in a repo with uncommitted work, nudge (at
    // most every 30 minutes a project). Not a turn you stopped.
    fn maybe_ship_it(&self, ev: &Value) {
        if s(ev, "kind") != Some("turn-end") || truthy(get(ev, "interrupted")) {
            return;
        }
        let home = self.home(s(ev, "sessionId").unwrap_or_default());
        let node = home.as_deref().and_then(|h| self.node_of(h));
        let nudge = {
            let mut g = lock(&self.inner);
            let target = match (&home, &node) {
                (Some(h), Some(n)) if truthy(n.get("repo")) => Some((h.as_str(), n.get("name").cloned().unwrap_or(Value::Null), n.get("repo").cloned().unwrap_or(Value::Null))),
                _ => None,
            };
            g.memory.ship_it(ev, target.as_ref().map(|(h, n, r)| (*h, n, r)), now_ms())
        };
        if let Some(n) = nudge {
            self.emit("activity", n);
        }
    }

    fn heard(&self, events: Vec<Event>) {
        let listening = self.listening.load(Ordering::SeqCst);
        for e in events {
            match e {
                // Refusals, from history as it's read too.
                Event::Refusal(v) => self.emit("refusal", v),
                Event::Activity(v) if listening => {
                    self.emit("activity", v.clone());
                    self.maybe_ship_it(&v);
                    self.queue_live();
                }
                Event::Tokens(v) if listening => self.emit("tokens", v),
                // A brand-new transcript means a new session: the map is
                // made again, a moment later.
                Event::NewFile(_) if listening => {
                    let n = self.new_files.fetch_add(1, Ordering::SeqCst) + 1;
                    if let Some(me) = self.arc() {
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(NEW_FILE_MS)).await;
                            if me.new_files.load(Ordering::SeqCst) == n {
                                let _ = me.refresh_map().await;
                            }
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// Starts reading: the transcripts' history, then the first refresh,
    /// and the timers that keep the map current.
    pub async fn begin(&self) -> Result<(), String> {
        let Some(me) = self.arc() else { return Err("XenoNet is stopping".into()) };
        let weak = self.me.clone();
        self.live.listen(Arc::new(move |events| {
            if let Some(s) = weak.upgrade() {
                s.heard(events);
            }
        }));
        self.live.start().await;
        self.listening.store(true, Ordering::SeqCst);
        let mut tasks = Vec::new();
        if let Some(demo) = self.demo().cloned() {
            tasks.push(tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_millis(DEMO_STEP_MS));
                every.tick().await;
                let mut k = 0usize;
                loop {
                    every.tick().await;
                    let d = demo.clone();
                    crate::util::blocking(move || d.step(k)).await;
                    k += 1;
                }
            }));
        }
        if let Err(e) = self.refresh_map().await {
            log("error", "state.refresh", &format!("refresh failed: {e}"));
        }
        let poll = self.cfg.poll();
        let m = me.clone();
        tasks.push(tokio::spawn(async move {
            let mut every = tokio::time::interval(poll);
            every.tick().await;
            loop {
                every.tick().await;
                if let Err(e) = m.refresh_map().await {
                    log("error", "state.refresh", &format!("refresh failed: {e}"));
                }
            }
        }));
        let m = me.clone();
        tasks.push(tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_millis(HEARTBEAT_MS));
            every.tick().await;
            loop {
                every.tick().await;
                let snap = m.snapshot();
                m.emit("live", snap);
            }
        }));
        let m = me.clone();
        tasks.push(tokio::spawn(async move {
            m.refresh_repos().await;
            let mut every = tokio::time::interval(Duration::from_millis(REPO_POLL_MS));
            every.tick().await;
            loop {
                every.tick().await;
                m.refresh_repos().await;
            }
        }));
        // A change in the wiki's folder makes the map again, once its
        // writes settle.
        if let Some(dir) = self.cfg.wiki_dir.clone() {
            let m = me.clone();
            tasks.push(tokio::spawn(async move {
                let look = |d: String| crate::util::blocking(move || wiki_stamp(&d));
                let mut last = look(dir.clone()).await;
                let mut every = tokio::time::interval(Duration::from_millis(WIKI_LOOK_MS));
                every.tick().await;
                loop {
                    every.tick().await;
                    if look(dir.clone()).await != last {
                        tokio::time::sleep(Duration::from_millis(WIKI_SETTLE_MS)).await;
                        last = look(dir.clone()).await;
                        let _ = m.refresh_map().await;
                    }
                }
            }));
        }
        // Build the search index in the background, then keep it current.
        let search = self.search.clone();
        tasks.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let mut every = tokio::time::interval(Duration::from_secs(60));
            loop {
                every.tick().await;
                search.index().await;
            }
        }));
        lock(&self.tasks).extend(tasks);
        Ok(())
    }

    pub fn end(&self) {
        for t in lock(&self.tasks).drain(..) {
            t.abort();
        }
        self.live.stop();
        self.codex.stop();
        self.search.close();
        self.store.flush();
    }

    // ---------- the projects folder ----------

    async fn detection(&self) -> Result<Value, CoreError> {
        let root = self.projects_dir();
        let codex = self.store.codex_picks(path_key(&root).as_deref());
        let current = match &codex {
            Some(_) => Some(json!(self.outline(&root).await?.1)),
            None => None,
        };
        let scan = lock(&self.inner).map.scan.clone();
        Ok(decide::project_detection(&root, scan.as_ref(), &self.store.project_rules(), codex.as_ref(), current.as_ref()))
    }

    // What Codex would be sent: the folders as XenoNet reads them on its
    // own rules and yours, without Codex's earlier picks.
    async fn outline(&self, root: &str) -> Result<(Vec<Value>, String), CoreError> {
        let dirs = lock(&self.inner).map.session_dirs.clone();
        let found = self.projects_scan.scan(root, dirs, self.project_rules(false, root), self.cfg.project_limits.clone(), true, None).await.map_err(err)?;
        let outline = classify::outline_folders(&found.folders);
        let fp = classify::fingerprint(&outline);
        Ok((outline, fp))
    }

    async fn after_change(&self) -> R<()> {
        self.refresh_after().await.map(|_| ()).map_err(err)
    }

    // ---------- transcripts ----------

    async fn transcript(&self, node: &Value, cursor: Option<&str>, limit: f64) -> Result<Value, String> {
        let raw = s(node, "rawId").unwrap_or_default().to_string();
        match &self.readers {
            Readers::Demo(d) => d.transcript(&raw),
            Readers::Real { claude } => {
                if s(node, "app") == Some("codex") {
                    let id = s(node, "id").unwrap_or_default().to_string();
                    let rollout = self.stats.files(&id).into_iter().next().map(|(f, _)| f);
                    self.codex.read_thread(&raw, cursor, limit, rollout).await
                } else {
                    claude.read(&raw, cursor, limit).await
                }
            }
        }
    }

    fn day(&self) -> Value {
        let history = self.live.history();
        let live = self.snapshot();
        let repos: Vec<Value> = {
            let projects = lock(&self.inner).map.projects.clone();
            projects
                .iter()
                .filter_map(|p| {
                    let key = p.get("key").map(js_string)?;
                    let st = self.repos.status_of(&key).filter(|st| truthy(st.get("dirty")))?;
                    let mut m = Map::new();
                    m.insert("name".into(), p.get("name").cloned().unwrap_or(Value::Null));
                    for (k, v) in st.as_object().into_iter().flatten() {
                        m.insert(k.clone(), v.clone());
                    }
                    Some(Value::Object(m))
                })
                .collect()
        };
        let mut title = |id: &Value| Ok::<Value, Infallible>(self.title(id));
        let mut home = |id: &Value| Ok::<Value, Infallible>(id.as_str().and_then(|i| self.home(i)).map_or(Value::Null, Value::String));
        let mut name = |id: &Value| {
            Ok::<Value, Infallible>(id.as_str().and_then(|i| self.node_of(i)).and_then(|n| n.get("name").cloned().filter(|v| !v.is_null())).unwrap_or_else(|| id.clone()))
        };
        let hooks = digest::DayHooks { title_of: Some(&mut title), home_of: Some(&mut home), name_of: Some(&mut name) };
        match digest::day_log(&history, now_ms(), hooks, Some(&live), &repos) {
            Ok(v) => v,
            Err(never) => match never {},
        }
    }

    fn salt(&self) -> String {
        redact::poster_salt(Some(&self.cfg.state_dir)).unwrap_or_default()
    }

    /// The live state's own simulations (server/index.js's simulate).
    fn simulate_live(&self, kind: &str, a: Option<String>, b: Option<String>) -> Option<Value> {
        use xenonet_sources::live::telemetry::MapOp;
        let now = now_ms();
        let (mut a, mut b) = (a, b);
        // Without sessions named, the most recently active ones on the map.
        if kind != "clear" && (a.is_none() || (kind == "collision" && b.is_none())) {
            let mut recent: Vec<(String, f64)> = self.live.ids().into_iter().filter(|(id, _)| self.has(id)).map(|(id, at)| (id, at.as_f64().unwrap_or(0.0))).collect();
            recent.sort_by(|x, y| y.1.total_cmp(&x.1));
            if a.is_none() {
                a = recent.first().map(|r| r.0.clone());
            }
            if b.is_none() {
                b = recent.iter().find(|r| Some(&r.0) != a.as_ref()).map(|r| r.0.clone());
            }
        }
        let mut t = lock(&self.live.telemetry);
        let ia = a.as_deref().and_then(|x| t.find(x));
        let ib = b.as_deref().and_then(|x| t.find(x));
        let mut acts: Vec<(String, Fields)> = Vec::new();
        let f = |pairs: Vec<(&str, Value)>| pairs.into_iter().map(|(k, v)| (k.to_string(), Some(v))).collect::<Vec<_>>();
        match kind {
            "needs-you" => {
                let i = ia?;
                let text = "Should I run the migration against the shared staging database, or only the local fixture?";
                let s = t.at_mut(i);
                s.set("question", Some(json!({ "callId": "sim-q", "text": text, "ts": num(now) })));
                s.map_op("inflight", MapOp::Set(Some(json!("sim-q")), Some(json!({ "name": "AskUserQuestion", "ts": num(now) }))));
                s.set("lastEventAt", Some(num(now)));
                acts.push((a.clone().unwrap_or_default(), f(vec![("kind", json!("needs-you")), ("detail", json!(text)), ("ts", num(now))])));
            }
            "collision" => {
                let (i, j) = (ia?, ib?);
                let file = std::path::Path::new(&self.projects_dir()).join("ledger").join("scripts").join("local-dev").join("app.ps1").to_string_lossy().into_owned();
                let _ = t.at_mut(i).add_edits(&[json!({ "path": file, "kind": "edit", "diff": "- $port = 3000\n+ $port = 3030\n  Start-Process node" })], now - 90e3);
                let _ = t
                    .at_mut(j)
                    .add_edits(&[json!({ "path": file, "kind": "update", "diff": "@@ -12,3 +12,4 @@\n-$port = 3000\n+$port = 3100\n+$env:NODE_ENV = 'development'" })], now - 20e3);
                t.at_mut(i).map_op("files", MapOp::Set(Some(json!(file)), Some(num(now - 90e3))));
                t.at_mut(j).map_op("files", MapOp::Set(Some(json!(file)), Some(num(now - 20e3))));
            }
            "stuck" => {
                let s = t.at_mut(ia?);
                s.reset_stall();
                s.set("question", Some(Value::Null));
                s.set("hookNeed", Some(Value::Null));
                s.map_op("inflight", MapOp::Clear);
                s.set("turn", Some(json!({ "active": true, "startedAt": num(now - 120e3) })));
                s.set("errorStreak", Some(json!(4)));
                s.set("lastErrorAt", Some(num(now)));
                s.set("errorSince", Some(num(now - 20e3)));
                for i in 0..4 {
                    s.push_tool(&json!({ "key": "shell|npm run db:push", "label": "npm run db:push", "loopable": true, "ok": false, "ts": num(now - f64::from(4 - i) * 5e3) }));
                }
                s.set("lastEventAt", Some(num(now)));
            }
            "turn-end" => {
                ia?;
                acts.push((a.clone().unwrap_or_default(), f(vec![("kind", json!("turn-end")), ("duration", json!(42000)), ("ts", num(now))])));
            }
            "wiki-write" => {
                ia?;
                acts.push((
                    a.clone().unwrap_or_default(),
                    f(vec![
                        ("kind", json!("tool")),
                        ("tool", json!("mcp__agent-wiki__wiki_log")),
                        ("category", json!("mcp")),
                        ("detail", json!("agent-wiki.wiki_log")),
                        ("server", json!("agent-wiki")),
                        ("pages", json!(["xenonet", "ledger"])),
                        ("files", json!([])),
                        ("ts", num(now)),
                    ]),
                ));
            }
            "clear" => {
                for id in t.ids() {
                    let Some(i) = t.find(&id) else { continue };
                    let s = t.at_mut(i);
                    let q = s.get("question").flatten();
                    if q.as_ref().and_then(|q| q.get("callId")).and_then(Value::as_str) == Some("sim-q") {
                        s.set("question", Some(Value::Null));
                        s.map_op("inflight", MapOp::Delete(Some(json!("sim-q"))));
                    }
                    s.set("errorStreak", Some(json!(0)));
                    let tools: Vec<Value> = items(s.get("recentTools").flatten().as_ref()).iter().filter(|x| s_of(x, "key") != Some("shell|npm run db:push")).cloned().collect();
                    s.set("recentTools", Some(Value::Array(tools)));
                    if let Some(Some(Value::Array(edits))) = s.map_op("edits", MapOp::Entries) {
                        for kv in edits {
                            let k = kv.get(0).cloned().unwrap_or(Value::Null);
                            if k.as_str().is_some_and(|p| p.replace('\\', "/").ends_with("local-dev/app.ps1")) {
                                s.map_op("edits", MapOp::Delete(Some(k)));
                            }
                        }
                    }
                }
            }
            _ => return None,
        }
        drop(t);
        for (id, ev) in acts {
            self.live.activity(&id, ev);
        }
        let mut out = json!({ "ok": true, "kind": kind });
        if kind != "clear" {
            out["a"] = a.map_or(Value::Null, Value::String);
        }
        if kind == "collision" {
            out["b"] = b.map_or(Value::Null, Value::String);
        }
        Some(out)
    }
}

// What the wiki's folder holds now, as one stamp: each file's path, size
// and mtime, three folders down.
fn wiki_stamp(dir: &str) -> String {
    let mut pick = |_: &str| true;
    xenonet_base::paths::list_files(dir, &mut pick, 3)
        .into_iter()
        .map(|f| match std::fs::metadata(&f) {
            Ok(m) => format!("{f}:{}:{}", m.len(), xenonet_base::fs::mtime_ms_of(&m).map_or(0.0, |t| t)),
            Err(_) => f,
        })
        .collect::<Vec<_>>()
        .join("|")
}

// What the assistant says of a reader's source (core/assistant/source.js's
// sourceOf): its gatherer's health and the live tailer's.
fn source_of(g: &Value, l: &Value, t: f64) -> Value {
    let state = s(g, "state").unwrap_or("idle");
    let last_ok = g.get("lastOk").and_then(Value::as_f64);
    let Some(last_ok) = last_ok.filter(|_| state != "idle") else { return json!({ "state": "unavailable", "observedAt": null, "coverage": "unknown", "reason": "not_started" }) };
    if state == "failed" || state == "stopped" {
        return json!({ "state": "unavailable", "observedAt": num(last_ok), "coverage": "unknown", "reason": if state == "stopped" { "stopped" } else { "read_failed" } });
    }
    let tailing = s(l, "state") == Some("running");
    let degraded = state == "degraded" || !tailing;
    let live_at = if tailing { Some(t) } else { l.get("lastOk").and_then(Value::as_f64) };
    let observed = live_at.map_or(last_ok, |at| last_ok.min(at));
    json!({ "state": if degraded { "degraded" } else { "ready" }, "observedAt": num(observed), "coverage": if degraded { "partial" } else { "complete" }, "reason": if degraded { "read_failed" } else { "ok" } })
}

fn s_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

#[async_trait]
impl State for MapState {
    fn sources(&self) -> R<Value> {
        Ok(lock(&self.inner).map.sources.clone())
    }
    fn blocks(&self) -> R<Value> {
        Ok(Value::Array(
            self.blocks
                .health()
                .into_iter()
                .map(|mut h| {
                    if let Some(m) = h.as_object_mut() {
                        m.shift_remove("budget");
                    }
                    h
                })
                .collect(),
        ))
    }
    fn graph(&self) -> R<Option<Value>> {
        Ok(lock(&self.inner).map.graph.clone())
    }
    async fn refresh(&self) -> R<Value> {
        self.refresh_map().await.map_err(err)
    }
    fn live_snapshot(&self) -> R<Value> {
        Ok(self.snapshot())
    }
    fn history_since(&self, since: f64) -> R<Value> {
        Ok(self.live.history_since(since))
    }
    async fn search_transcripts(&self, q: &str) -> R<Value> {
        let (last, indexing) = {
            let st = lock(&self.search.status);
            (st.last_index_at, st.indexing)
        };
        if now_ms() - last > 15000.0 && !indexing {
            self.search.index().await;
        }
        let hits = self.search.search(q, 30.0).await.map_err(err)?;
        Ok(Value::Array(
            hits.into_iter()
                .map(|mut r| {
                    let id = r.get("sessionId").cloned().unwrap_or(Value::Null);
                    r["inGraph"] = json!(id.as_str().is_some_and(|i| self.has(i)));
                    r["title"] = self.title(&id);
                    r
                })
                .collect(),
        ))
    }
    fn search_status(&self) -> R<Value> {
        Ok(self.search.status_json())
    }
    fn node(&self, id: &str) -> R<Option<Value>> {
        Ok(self.node_of(id))
    }
    fn nodes(&self) -> R<Vec<Value>> {
        Ok(self.all_nodes())
    }
    fn home_of(&self, id: &str) -> R<Value> {
        Ok(self.home(id).map_or(Value::Null, Value::String))
    }
    fn title_of(&self, id: Option<&str>) -> R<Value> {
        Ok(self.title(&id.map_or(Value::Null, |i| json!(i))))
    }
    fn collision_diffs(&self, a: Option<&str>, b: Option<&str>) -> R<Value> {
        Ok(self.live.collision_diffs(a.unwrap_or("undefined"), b.unwrap_or("undefined")))
    }
    async fn handoff(&self, node: Option<&str>, target: &str) -> R<Value> {
        let n = node.and_then(|id| self.node_of(id)).filter(|n| s(n, "kind") == Some("session")).ok_or_else(|| err("unknown session"))?;
        let id = s(&n, "id").unwrap_or_default().to_string();
        let mut notes = self.live.session_notes(&id);
        if notes.is_null() {
            notes = json!({ "prompts": [], "messages": [], "files": [], "edits": [] });
        }
        if decide::handoff_needs_transcript(&notes) {
            let limit = if s(&n, "app") == Some("codex") { 60.0 } else { 80.0 };
            let page = self.transcript(&n, None, limit).await.map_err(err)?;
            notes = decide::handoff_notes(&notes, Some(&page));
        }
        let folder = n.get("worktree").filter(|w| truthy(Some(w))).or_else(|| n.get("cwd").filter(|c| truthy(Some(c)))).cloned().unwrap_or(Value::Null);
        Ok(json!({ "brief": digest::handoff_brief(&n, &notes, Some(target)), "folder": folder }))
    }
    fn digest(&self, since: f64) -> R<Value> {
        let events = self.live.history_since(since);
        let live = self.snapshot();
        let mut title = |id: &Value| Ok::<Value, Infallible>(self.title(id));
        match digest::summarize(items(Some(&events)), since, now_ms(), &mut title, Some(&live)) {
            Ok(v) => Ok(v),
            Err(never) => match never {},
        }
    }
    fn day_log(&self) -> R<Value> {
        Ok(self.day())
    }
    fn constellation(&self, names: bool) -> R<Value> {
        let day = self.day();
        Ok(redact::constellation_of(Some(&day), &self.salt(), now_ms(), names, &self.all_nodes()))
    }
    fn export_labels(&self) -> R<Value> {
        let day = self.day();
        Ok(redact::labels_of(&self.all_nodes(), Some(&day), &self.salt(), now_ms()))
    }
    async fn narrate(&self, kind: &str, since: f64) -> R<Value> {
        if self.cfg.demo {
            return Ok(json!(xenonet_app::demo::NARRATION));
        }
        let text = if kind == "day" { digest::summary_text(&self.day(), "Today so far") } else { digest::summary_text(&self.digest(since)?, "While the user was away") };
        crate::narrate::ask_codex(&self.cfg.codex_binary(), &digest::narrate_prompt(&text), None).await.map(Value::String).map_err(err)
    }
    async fn project_detection(&self) -> R<Value> {
        self.detection().await
    }
    async fn set_project_rule(&self, dir: Value, as_: Value) -> R<Value> {
        let full = xenonet_base::paths::resolve(&match &dir {
            Value::Null => String::new(),
            v => js_string(v),
        });
        let is_dir = truthy(Some(&as_)) && std::fs::metadata(&full).is_ok_and(|m| m.is_dir());
        if let Some(problem) = decide::rule_problem(Some(&as_), &full, &self.projects_dir(), is_dir) {
            return Err(err(problem));
        }
        let key = path_key(&full).unwrap_or_default();
        self.store.set_project_rule(&key, truthy(Some(&as_)).then(|| json!({ "path": full, "as": as_, "at": num(now_ms()) })));
        self.after_change().await?;
        self.detection().await
    }
    async fn clear_project_rules(&self, all: bool) -> R<Value> {
        let key = path_key(&self.projects_dir());
        self.store.clear_project_rules(if all { None } else { key.as_deref() });
        self.after_change().await?;
        self.detection().await
    }
    async fn rescan_projects(&self) -> R<Value> {
        self.projects_scan.forget();
        self.after_change().await?;
        self.detection().await
    }
    async fn codex_outline(&self) -> R<Value> {
        let (outline, fp) = self.outline(&self.projects_dir()).await?;
        Ok(json!({ "outline": outline, "fingerprint": fp }))
    }
    async fn ask_codex_about_projects(&self, expected: Value, decider: bool) -> R<Value> {
        let root = self.projects_dir();
        let (outline, fp) = self.outline(&root).await?;
        if truthy(Some(&expected)) && expected != json!(fp) {
            return Err(status(409, "The folders changed since you looked. Look again before sending."));
        }
        let started = now_ms();
        let picks = if self.cfg.demo {
            xenonet_app::demo::demo_picks(&Value::Array(outline.clone()))
        } else if decider {
            match self.decider_picks(&outline).await {
                Ok(p) => p,
                Err(e) if e.is_pending() => return Err(e),
                Err(e) => return Err(status(if e.status == Some(409) { 409 } else { 502 }, format!("The decision model couldn't sort the folders: {}", e.message))),
            }
        } else {
            match crate::narrate::ask_codex(&self.cfg.codex_binary(), &classify::picks_prompt(&outline), Some(classify::picks_schema()))
                .await
                .and_then(|text| classify::parse_picks(&text, &outline))
            {
                Ok(p) => Value::Array(p),
                Err(e) => return Err(status(502, format!("Codex couldn't sort the folders: {e}"))),
            }
        };
        // Who sorted them: Codex, or the decision model by its name.
        let by = match (decider, self.emitter.ctx()) {
            (true, Some(ctx)) => ctx.cores.decider.status().await.ok().and_then(|s| s.get("model").cloned()).unwrap_or(Value::Null),
            (true, None) => Value::Null,
            (false, _) => json!("codex"),
        };
        let at = now_ms();
        self.store
            .set_codex_picks(
                path_key(&root).as_deref(),
                Some(json!({ "ranAt": num(at), "ms": num(at - started), "fingerprint": fp, "sent": outline.len(), "picks": picks, "by": by })),
            )
            .map_err(err)?;
        self.after_change().await?;
        self.detection().await
    }
    async fn forget_codex_picks(&self) -> R<Value> {
        self.store.set_codex_picks(path_key(&self.projects_dir()).as_deref(), None).map_err(err)?;
        self.after_change().await?;
        self.detection().await
    }
    fn identity_detection(&self) -> R<Value> {
        let (at, ids) = {
            let g = lock(&self.inner);
            let gr = g.map.graph.as_ref();
            (gr.and_then(|gr| gr.get("generatedAt")).cloned(), gr.and_then(|gr| gr.get("identities")).cloned().unwrap_or_else(|| json!([])))
        };
        let saved = self.store.identities();
        let mut node = |id: &str| self.node_of(id);
        Ok(decide::identity_detection(self.identities.is_some(), at.as_ref(), Some(&ids), &saved, &mut node))
    }
    async fn name_identity(&self, id: Value, name: Value) -> R<Value> {
        let known = self.known(&id)?;
        self.store.set_identity_name(&known, decide::clean_identity_name(Some(&name)));
        self.after_change().await?;
        self.identity_detection()
    }
    async fn hide_identity(&self, id: Value, hidden: Value) -> R<Value> {
        let known = self.known(&id)?;
        self.store.set_identity_hidden(&known, truthy(Some(&hidden)));
        self.after_change().await?;
        self.identity_detection()
    }
    async fn pin_identity(&self, node: Value, identity: Value) -> R<Value> {
        let id = match &node {
            Value::String(s) => s.clone(),
            Value::Null => "null".into(),
            v => js_string(v),
        };
        let kind = match self.node_of(&id) {
            Some(n) => n.get("kind").and_then(Value::as_str).map(str::to_string),
            None => self.store.identities().get("pins").and_then(|p| p.get(&id)).filter(|p| truthy(Some(p))).map(|_| "pinned".to_string()),
        };
        if let Some(problem) = decide::pin_problem(kind.as_deref()) {
            return Err(err(problem));
        }
        let to = if truthy(Some(&identity)) { Some(self.known(&identity)?) } else { None };
        self.store.set_identity_pin(&id, to);
        self.after_change().await?;
        self.identity_detection()
    }
    async fn rescan_identities(&self) -> R<Value> {
        if let Some(w) = &self.identities {
            w.forget();
            let projects = lock(&self.inner).map.projects.clone();
            w.refresh_repos(&projects).await;
        }
        self.after_change().await?;
        self.identity_detection()
    }
    fn summary(&self) -> R<Value> {
        let snap = self.snapshot();
        let mut title = |id: &Value| self.title(id);
        Ok(decide::summary(&snap, &mut title, &decide::url(&self.cfg.host, &self.cfg.port)))
    }
    async fn session_stats(&self, id: &str) -> R<Option<Value>> {
        self.stats.stats(id).await.map_err(err)
    }
    async fn read_transcript(&self, node: &Value, cursor: Option<&str>, limit: f64) -> R<Value> {
        self.transcript(node, cursor, limit).await.map_err(err)
    }
    async fn rename_session(&self, node: Value, title: Value) -> R<Value> {
        let n = node.as_str().and_then(|id| self.node_of(id));
        if !rename::renamable(n.as_ref()) {
            return Err(status(404, "XenoNet doesn't know that session"));
        }
        let n = n.unwrap_or(Value::Null);
        let name = rename::clean_title(title.as_str().unwrap_or(""));
        if name.is_empty() {
            return Err(status(400, "A session's name can't be empty"));
        }
        let id = s(&n, "id").unwrap_or_default().to_string();
        let mut saved = Vec::new();
        let mut note = Value::Null;
        match &self.readers {
            Readers::Demo(_) => {
                let mut g = lock(&self.inner);
                match g.demo_titles.iter_mut().find(|(k, _)| *k == id) {
                    Some(slot) => slot.1 = json!(name),
                    None => g.demo_titles.push((id.clone(), json!(name))),
                }
            }
            Readers::Real { claude } => {
                if s(&n, "app") == Some("codex") {
                    self.codex.rename(s(&n, "rawId").unwrap_or_default(), &name).await.map_err(err)?;
                    saved.push("Codex");
                } else {
                    let done = claude.rename(&n, &name).await.map_err(err)?;
                    saved.push("Claude Code");
                    if done.desktop {
                        saved.push("Claude desktop");
                    }
                    note = decide::rename_note(done.desktop, done.desktop_error.map(Value::String).as_ref());
                }
            }
        }
        self.after_change().await?;
        let shown = self.node_of(&id).and_then(|n| n.get("title").cloned()).filter(|t| !t.is_null()).unwrap_or_else(|| json!(name));
        Ok(json!({ "id": id, "title": shown, "saved": saved, "note": note, "demo": self.cfg.demo }))
    }
    fn note_hook(&self, id: &str, body: &Value) -> R<()> {
        self.live.note_hook(id, body).map_err(err)
    }
    fn root_info(&self) -> R<Value> {
        let mut r = workspace::root(self.cfg.env_projects_dir.as_deref(), Some(&self.store.workspace_root()), &self.cfg.home_dir);
        r["scan"] = lock(&self.inner).map.scan.clone().unwrap_or(Value::Null);
        Ok(r)
    }
    async fn set_root(&self, path: Value) -> R<Value> {
        if let Some(why) = workspace::locked(self.cfg.env_projects_dir.as_deref(), &self.cfg.home_dir) {
            return Err(status(409, why));
        }
        if let Some(d) = self.demo() {
            d.clients_ready().await;
        }
        let full = if path.is_null() {
            None
        } else {
            let v = workspace::validate(Some(&path), &self.cfg.home_dir, &self.cfg.platform);
            if v["ok"] != true {
                return Err(status(400, v["error"].as_str().unwrap_or("That folder can't be read").to_string()));
            }
            v["path"].as_str().map(str::to_string)
        };
        let before = self.projects_dir();
        let default = workspace::root(self.cfg.env_projects_dir.as_deref(), Some(&self.store.workspace_root()), &self.cfg.home_dir)["default"].as_str().unwrap_or("").to_string();
        self.store.set_workspace_root(workspace::chosen(full.as_deref(), &default));
        let after = workspace::root(self.cfg.env_projects_dir.as_deref(), Some(&self.store.workspace_root()), &self.cfg.home_dir)["path"].as_str().unwrap_or("").to_string();
        let changed = path_key(&before) != path_key(&after);
        {
            let mut g = lock(&self.inner);
            g.projects_dir = after.clone();
            if changed {
                g.repo_pass_for = path_key(&after);
            }
        }
        if changed {
            self.projects_scan.forget();
            self.after_change().await?;
        }
        let mut out = self.root_info()?;
        out["changed"] = json!(changed);
        Ok(out)
    }
    async fn check_root(&self, path: Value) -> R<Value> {
        if let Some(d) = self.demo() {
            d.clients_ready().await;
        }
        let markers: Vec<&str> = xenonet_state::projects::VCS_MARKERS.to_vec();
        let start = workspace::check_start(Some(&path), &self.cfg.home_dir, &self.projects_dir(), &self.cfg.platform, &markers);
        if let Some(done) = start.get("done") {
            return Ok(done.clone());
        }
        let full = workspace::resolve_folder(Some(&path), &self.cfg.home_dir);
        let rules = full.as_deref().map_or_else(|| json!({}), |f| self.project_rules(true, f));
        let dir = start["path"].as_str().unwrap_or("").to_string();
        let dirs = lock(&self.inner).map.session_dirs.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let scan = self.projects_scan.scan(&dir, dirs, rules, self.cfg.project_limits.clone(), true, Some(stop.clone()));
        let found = match tokio::time::timeout(Duration::from_millis(5000), scan).await {
            Ok(Ok(f)) => Some(f.scan),
            Ok(Err(_)) => None,
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                None
            }
        };
        Ok(workspace::check_finish(&start, found.as_ref(), 5000.0))
    }
    fn simulate(&self, kind: &str, a: Option<String>, b: Option<String>) -> R<Option<Value>> {
        Ok(self.simulate_live(kind, a, b))
    }
    fn someone_waits(&self, now: f64) -> R<bool> {
        Ok(self.live.someone_waits(now))
    }
    async fn refresh_after_change(&self) -> R<()> {
        self.after_change().await
    }
    fn live_ids(&self) -> R<Vec<(String, Value)>> {
        Ok(self.live.ids())
    }
    fn live_info(&self, id: &str, now: f64) -> R<Option<Value>> {
        if let Some(i) = self.live.info(id, now) {
            return Ok(Some(i));
        }
        Ok(self.observed(id, now))
    }
    fn observed_live(&self, id: &str) -> R<bool> {
        Ok(self.observed(id, now_ms()).is_some())
    }
    fn identities(&self) -> R<Vec<Value>> {
        Ok(lock(&self.inner).map.graph.as_ref().map(|g| items(g.get("identities")).to_vec()).unwrap_or_default())
    }
    fn recent_session(&self) -> R<Option<String>> {
        let mut live: Vec<(String, f64)> = self.live.ids().into_iter().filter(|(id, _)| self.has(id)).map(|(id, at)| (id, at.as_f64().unwrap_or(0.0))).collect();
        live.sort_by(|x, y| y.1.total_cmp(&x.1));
        if let Some((id, _)) = live.into_iter().next() {
            return Ok(Some(id));
        }
        Ok(self.all_nodes().into_iter().find(|n| s(n, "kind") == Some("session")).and_then(|n| n.get("id").and_then(Value::as_str).map(str::to_string)))
    }
    fn assistant_sources(&self, now: f64) -> R<Value> {
        use crate::blocks::Block;
        let tail = self.live.block().health();
        Ok(json!({ "claude": source_of(&self.claude_sessions.health(), &tail, now), "codex": source_of(&self.codex_threads.health(), &tail, now) }))
    }
    fn graph_version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }
    fn forget_layout(&self, prefix: &str) -> R<usize> {
        let gone: Vec<String> = self.store.layout().as_object().map(|m| m.keys().filter(|k| k.starts_with(prefix)).cloned().collect()).unwrap_or_default();
        if !gone.is_empty() {
            self.store.change(|d| {
                let layout = crate::store::obj_at(d, &["layout"]);
                for k in &gone {
                    layout.shift_remove(k);
                }
            });
        }
        Ok(gone.len())
    }
    async fn start(&self) -> R<()> {
        self.begin().await.map_err(err)
    }
    fn stop(&self) {
        self.end();
    }
    fn session_costs(&self, now: f64) -> R<Vec<Value>> {
        Ok(items(Some(&lock(&self.live.telemetry).session_costs(now))).to_vec())
    }
    fn add_block(&self, health: Arc<dyn Fn() -> Value + Send + Sync>, source: Option<String>) -> R<()> {
        self.blocks.add(Arc::new(Reported(health)), source.as_deref()).map_err(err)
    }
}

impl MapState {
    // The decision model's picks (classify.js's askDeciderForPicks): the
    // outline as one choice a folder, a batch at a time.
    async fn decider_picks(&self, outline: &[Value]) -> R<Value> {
        let Some(ctx) = self.emitter.ctx() else { return Err(CoreError::pending("core/decide.js")) };
        let state = json!({ "about": classify::DECIDER_ABOUT, "folders": outline });
        let mut picks = Vec::new();
        let mut offset = 0.0;
        for batch in classify::jev_questions(outline) {
            let r = ctx.cores.decider.decide(json!({ "kind": "projects", "state": state, "questions": batch.get("questions").cloned().unwrap_or(Value::Null) })).await?;
            let folders = items(batch.get("folders")).to_vec();
            picks.extend(classify::picks_from_answers(r.get("answers").unwrap_or(&Value::Null), &folders, offset));
            offset += folders.len() as f64;
        }
        Ok(Value::Array(picks))
    }

    fn known(&self, id: &Value) -> R<String> {
        let ids = lock(&self.inner).map.graph.as_ref().map(|g| items(g.get("identities")).to_vec()).unwrap_or_default();
        let known = self.store.identities().get("known").cloned().unwrap_or_else(|| json!({}));
        decide::known_identity(Some(id), &ids, &known).ok_or_else(|| err("XenoNet doesn't know that identity"))
    }

    // A run an actor reported, or another observed one, as the engine reads
    // a live session.
    fn observed(&self, id: &str, now: f64) -> Option<Value> {
        if let Some(l) = self.observed.iter().find_map(|o| o.live(id)) {
            return Some(l);
        }
        let runs = lock(&self.runs);
        let r = runs.runs.get(id)?;
        let st = runs.status_of(id, r.serial, (r.ended_at, r.updated_at), now);
        Some(obj(vec![
            ("observed", json!(true)),
            ("lastEventAt", num(r.updated_at)),
            ("endedAt", r.ended_at.map_or(Value::Null, num)),
            ("status", json!(st)),
            ("edits", json!([])),
        ]))
    }
}

/// Runs an actor reports from elsewhere (state.reportedRuns).
pub struct RunsCore(pub Arc<MapState>);

impl Runs for RunsCore {
    fn report(&self, actor: &str, run: Value) -> Value {
        let text = |k: &str| run.get(k).and_then(Value::as_str).map(str::to_string);
        let app = run.get("app").filter(|a| !a.is_null()).map(js_string).unwrap_or_default();
        // presence = "working" when it isn't given.
        let presence = match run.get("presence") {
            None => Some("working".to_string()),
            Some(p) => p.as_str().map(str::to_string),
        };
        let mut now = now_ms;
        match lock(&self.0.runs).report(actor, text("run").as_deref(), &app, text("title").as_deref(), presence.as_deref(), &mut now) {
            Ok(id) => json!({ "ok": true, "id": id }),
            Err(error) => json!({ "ok": false, "error": error }),
        }
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
