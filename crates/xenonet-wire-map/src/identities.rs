//! core/identities.js's watch: the accounts work happens under. What each
//! app's files say (the Claude and ChatGPT sign-ins, gh's hosts.yml, ssh
//! aliases), each repo's git identity from `git config`, and the links on
//! the map, are xenonet-state's identities.rs; this reads the files again
//! only when they change, runs git config, and keeps the watch's health.
//! Nothing here leaves the machine.

use crate::blocks::{health_of, Block};
use crate::gatherer::{Gather, Options};
use crate::store::{obj_at, StateStore};
use crate::util::{lock, now_ms, s};
use crate::vendor::Vendor;
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_state::identities::{self as ids, LinkInput, Signin, TranscriptOwners};

/// Where the watch reads.
#[derive(Clone, Debug)]
pub struct Files {
    pub home: String,
    pub claude_config_file: String,
    pub codex_home: String,
    pub gh_config_dir: String,
    pub claude_projects_dir: String,
}

struct Repo {
    path: String,
    origin: Value,
    source: Arc<Gather>,
}

struct Inner {
    life: Lifecycle,
    /// file → its source, read again when its mtime changes.
    files: Vec<(String, Arc<Gather>)>,
    /// project key → its repo's git identity.
    repos: Vec<(String, Repo)>,
    owners: TranscriptOwners,
}

pub struct IdentityWatch {
    at: Files,
    vendor: Arc<Vendor>,
    /// The environment git config runs in: away from your own home folder
    /// (the demo, tests), it reads that folder's config, not yours.
    git_env: Option<Map<String, Value>>,
    inner: Mutex<Inner>,
    /// One pass over the repos at a time.
    pass: tokio::sync::Mutex<()>,
}

fn mtime(file: &str) -> Option<String> {
    let m = std::fs::metadata(file).ok()?;
    xenonet_base::fs::mtime_ms_of(&m).ok().map(xenonet_base::json::number_string)
}

// JSON.parse for another app's file, whose error says where the text goes
// wrong and never quotes it (it can hold a token).
fn parse_json(text: &str) -> Result<Value, String> {
    xenonet_base::json::parse(text).map_err(|_| match xenonet_schema::vendor::json_error_at(text) {
        Some((line, column)) => format!("isn't valid JSON (line {line}, column {column})"),
        None => "isn't valid JSON".into(),
    })
}

impl IdentityWatch {
    pub fn new(at: Files, vendor: Arc<Vendor>, env: &Map<String, Value>) -> Arc<IdentityWatch> {
        let yours = std::env::var("HOME").unwrap_or_default();
        let git_env = ids::git_env(env, &at.home, &yours);
        let owners = TranscriptOwners::new(&at.claude_projects_dir);
        Arc::new(IdentityWatch {
            at,
            vendor,
            git_env,
            inner: Mutex::new(Inner { life: Lifecycle::new(now_ms()), files: Vec::new(), repos: Vec::new(), owners }),
            pass: tokio::sync::Mutex::new(()),
        })
    }

    /// "Detect identities again": nothing kept.
    pub fn forget(&self) {
        let mut g = lock(&self.inner);
        g.files.clear();
        g.repos.clear();
        g.owners = TranscriptOwners::new(&self.at.claude_projects_dir);
    }

    // What a file says, parsed again only when its mtime changes. A file
    // that isn't there says nothing, and so does one that can't be read or
    // parsed, until it's written again.
    fn file(&self, file: &str, parse: impl FnOnce(&str) -> Result<Value, String>) -> Value {
        let source = {
            let mut g = lock(&self.inner);
            match g.files.iter().find(|(f, _)| f == file) {
                Some((_, s)) => s.clone(),
                None => {
                    let name = Path::new(file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let s = Arc::new(Gather::new(&format!("identities:{name}"), Options { stamped: true, fallback: Value::Null, ..Default::default() }));
                    g.files.push((file.to_string(), s.clone()));
                    s
                }
            }
        };
        let f = file.to_string();
        let value = source.get(false, mtime(file), move || match std::fs::read_to_string(&f) {
            Ok(text) => parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
            Err(e) => Err(e.to_string()),
        });
        self.report();
        value
    }

    /// What each app says: { claude, chatgpt, gh, ssh, claudeFile,
    /// chatgptFile }, with ssh's aliases as [[alias, host]…].
    pub fn accounts(&self) -> Value {
        let chatgpt_file = Path::new(&self.at.codex_home).join("auth.json").to_string_lossy().into_owned();
        let v = &self.vendor;
        let claude = self.file(&self.at.claude_config_file, |t| {
            let data = parse_json(t)?;
            Ok(ids::claude_account(v.check("anthropic/claude-code/config", data, None, None).as_ref()))
        });
        let chatgpt = self.file(&chatgpt_file, |t| {
            let auth = parse_json(t)?;
            let claims = ids::id_token_claims(Some(&auth));
            let claims = if claims.is_null() { claims } else { v.check("openai/codex/auth-claims", claims, None, None).unwrap_or(Value::Null) };
            Ok(ids::chatgpt_account(Some(&auth), Some(&claims)))
        });
        let gh = self.file(&Path::new(&self.at.gh_config_dir).join("hosts.yml").to_string_lossy(), |t| {
            v.check("github/gh/hosts", ids::gh_hosts_shape(t), None, None);
            Ok(Value::Array(ids::parse_gh_hosts(t)))
        });
        let ssh = self.file(&Path::new(&self.at.home).join(".ssh").join("config").to_string_lossy(), |t| Ok(json!(ids::parse_ssh_aliases(t))));
        json!({
            "claude": claude,
            "chatgpt": chatgpt,
            "gh": if gh.is_null() { json!([]) } else { gh },
            "ssh": if ssh.is_null() { json!([]) } else { ssh },
            "claudeFile": self.at.claude_config_file,
            "chatgptFile": chatgpt_file,
        })
    }

    fn global_stamp(&self) -> String {
        let xdg = self.git_env.as_ref().and_then(|e| e.get("XDG_CONFIG_HOME")).and_then(Value::as_str).map(str::to_string).or_else(|| std::env::var("XDG_CONFIG_HOME").ok());
        ids::global_stamp(&self.at.home, xdg.as_deref())
    }

    async fn git_config(&self, dir: &str) -> Option<Value> {
        let args: Vec<String> = ["-C", dir, "config", "--get-regexp", "^(user\\.(name|email)|credential\\..*)$"].iter().map(|s| s.to_string()).collect();
        let (env, clear): (Vec<(String, String)>, bool) = match &self.git_env {
            Some(e) => (e.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect(), true),
            None => (Vec::new(), false),
        };
        let ran = crate::git::run_git(&args, &env, clear, Duration::from_secs(8)).await;
        // Exit status 1 just means nothing matched.
        (ran.ok || ran.code == Some(1)).then(|| ids::parse_git_config(&ran.stdout))
    }

    /// Each repo's git identity, read again when its config or yours
    /// changes. Says whether anything changed.
    pub async fn refresh_repos(&self, projects: &[Value]) -> bool {
        if !crate::git::git_available() {
            return false;
        }
        let _one = self.pass.lock().await;
        let global = self.global_stamp();
        let ssh: Vec<(String, String)> =
            crate::util::items(self.accounts().get("ssh")).iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))).collect();
        let repos: Vec<&Value> = projects.iter().filter(|p| xenonet_base::text::truthy(p.get("git")) && s(p, "role") != Some("group")).collect();
        let mut changed = false;
        // Four at a time in the JS; one after another here, which reads the
        // same: each is a git config of its own.
        for p in &repos {
            let key = s(p, "key").unwrap_or_default().to_string();
            let path = s(p, "path").unwrap_or_default().to_string();
            let origin = p.get("git").and_then(|g| g.get("origin")).cloned().unwrap_or(Value::Null);
            let source = {
                let mut g = lock(&self.inner);
                match g.repos.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, r)) => {
                        r.path = path.clone();
                        r.origin = origin.clone();
                        r.source.clone()
                    }
                    None => {
                        let source = Arc::new(Gather::new("identities:repo", Options { stamped: true, retry: true, keep: true, ..Default::default() }));
                        g.repos.push((key.clone(), Repo { path: path.clone(), origin: origin.clone(), source: source.clone() }));
                        source
                    }
                }
            };
            let stamp = format!("{global}|{}|{}", ids::repo_config_mtime(&path).map_or_else(|| "null".into(), xenonet_base::json::number_string), origin.as_str().unwrap_or(""));
            let before = { lock(&source.state).good() }.then(|| source.value());
            let home = self.at.home.clone();
            let p2 = path.clone();
            let ssh2 = ssh.clone();
            let origin2 = origin.clone();
            let value = source
                .run(false, Some(stamp), || async move {
                    let cfg = self.git_config(&p2).await.ok_or_else(|| ids::unreadable_config(&p2, &home))?;
                    let push = ids::push_account(Some(&origin2), Some(&cfg), &ssh2).map_err(|e| e.0)?;
                    Ok(json!({ "name": cfg.get("name").cloned().unwrap_or(Value::Null), "email": cfg.get("email").cloned().unwrap_or(Value::Null), "push": push }))
                })
                .await;
            if source.last().is_some_and(|l| l.ok) && Some(&value) != before.as_ref() {
                changed = true;
            }
        }
        lock(&self.inner).repos.retain(|(k, _)| repos.iter().any(|p| s(p, "key") == Some(k.as_str())));
        self.report();
        changed
    }

    // Degraded while a source couldn't be read the last time it was, and
    // running again once they all can.
    fn report(&self) {
        let mut g = lock(&self.inner);
        let file = g.files.iter().find_map(|(f, s)| s.last().filter(|l| !l.ok).map(|l| (f.clone(), l.error.unwrap_or_default())));
        let repo = g.repos.iter().find_map(|(_, r)| r.source.last().filter(|l| !l.ok).map(|l| l.error.unwrap_or_default()));
        let state = g.life.state.as_str().to_string();
        let last = g.life.last_error.as_ref().map(|e| e.message.clone());
        let (say, problem) = ids::report(file.as_ref().map(|(f, e)| (f.as_str(), e.as_str())), repo.as_deref(), &self.at.home, &state, last.as_deref());
        match say {
            "ok" => g.life.ok(now_ms()),
            "problem" => g.life.problem(problem.as_deref().unwrap_or_default(), now_ms()),
            _ => {}
        }
    }

    /// Links the map's sessions and projects to identities, and keeps what
    /// that changes of the saved state. Gives the identities, and whether
    /// the saved state changed.
    pub fn link(&self, graph: &mut Value, store: &StateStore) -> Result<Vec<Value>, String> {
        let now = now_ms();
        let saved = store.identities();
        let nodes = crate::util::items(graph.get("nodes")).to_vec();
        let mut owners: Vec<(Value, Value)> = Vec::new();
        {
            let mut g = lock(&self.inner);
            for n in &nodes {
                let id = s(n, "id").unwrap_or_default();
                let pinned = saved.get("pins").and_then(|p| p.get(id)).is_some_and(|p| xenonet_base::text::truthy(Some(p)));
                if s(n, "kind") == Some("session") && s(n, "app") == Some("claude") && !xenonet_base::text::truthy(n.get("claudeAccount")) && !pinned {
                    owners.push((json!(id), g.owners.owner(id, now).unwrap_or(Value::Null)));
                }
            }
        }
        let repos: Vec<(Value, Value)> = {
            let g = lock(&self.inner);
            g.repos.iter().filter(|(_, r)| lock(&r.source.state).good()).map(|(k, r)| (json!(k), r.source.value())).collect()
        };
        let accounts = self.accounts();
        let ssh: Vec<(String, String)> =
            crate::util::items(accounts.get("ssh")).iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))).collect();
        let node_input: Vec<Value> = nodes
            .iter()
            .map(|n| {
                let mut m = Map::new();
                for k in ["id", "kind", "app", "claudeAccount", "provider", "updatedAt", "createdAt"] {
                    if let Some(v) = n.get(k) {
                        m.insert(k.into(), v.clone());
                    }
                }
                if let Some(o) = n.get("git").and_then(|g| g.get("origin")) {
                    m.insert("origin".into(), o.clone());
                }
                for k in ["key", "group", "name"] {
                    if let Some(v) = n.get(k) {
                        m.insert(k.into(), v.clone());
                    }
                }
                Value::Object(m)
            })
            .collect();
        let links: Vec<(Value, Value, Value)> = crate::util::items(graph.get("links"))
            .iter()
            .map(|l| (l.get("source").cloned().unwrap_or(Value::Null), l.get("target").cloned().unwrap_or(Value::Null), l.get("kind").cloned().unwrap_or(Value::Null)))
            .collect();
        let account_input = json!({ "claude": accounts["claude"], "chatgpt": accounts["chatgpt"], "gh": accounts["gh"], "claudeFile": accounts["claudeFile"], "chatgptFile": accounts["chatgptFile"] });
        let home = Some(self.at.home.as_str()).filter(|h| !h.is_empty());
        let out =
            ids::link_identities(LinkInput { nodes: &node_input, links: &links, accounts: &account_input, ssh: &ssh, repos: &repos, owners: &owners, saved: &saved, home, now })
                .map_err(|e| e.0)?;
        if let Some(Value::Array(list)) = graph.get_mut("nodes") {
            for (i, link) in out.ran_as {
                if let Some(n) = list.get_mut(i).and_then(Value::as_object_mut) {
                    n.insert("ranAs".into(), link);
                }
            }
            for (i, links) in out.identities {
                if let Some(n) = list.get_mut(i).and_then(Value::as_object_mut) {
                    n.insert("identities".into(), links);
                }
            }
        }
        // What linking changed of the saved state: the sign-in timelines
        // and what's remembered of each identity.
        store.with(|d| {
            let signins = obj_at(d, &["identities", "signins"]);
            for (app, op) in &out.signins {
                let spans = signins.entry(app.to_string()).or_insert_with(|| json!([]));
                if !spans.is_array() {
                    *spans = json!([]);
                }
                if let Some(list) = spans.as_array_mut() {
                    match op {
                        Signin::Same => {}
                        Signin::Extend(to) => {
                            if let Some(last) = list.last_mut().and_then(Value::as_object_mut) {
                                last.insert("to".into(), crate::util::num(*to));
                            }
                        }
                        Signin::Push { span, drop } => {
                            list.push(span.clone());
                            let n = (*drop).min(list.len());
                            list.drain(..n);
                        }
                    }
                }
            }
            let known = obj_at(d, &["identities", "known"]);
            for (id, whole, value) in &out.known {
                if *whole {
                    known.insert(id.clone(), value.clone());
                } else if let (Some(Value::Object(k)), Value::Object(v)) = (known.get_mut(id), value) {
                    for (a, b) in v {
                        k.insert(a.clone(), b.clone());
                    }
                }
            }
        });
        if out.dirty {
            store.save();
        }
        if let Some(g) = graph.as_object_mut() {
            g.insert("identities".into(), Value::Array(out.list.clone()));
        }
        Ok(out.list)
    }
}

impl Block for IdentityWatch {
    fn name(&self) -> String {
        "identities".into()
    }
    fn health(&self) -> Value {
        let g = lock(&self.inner);
        let runs = |list: Vec<&Arc<Gather>>| list.iter().map(|s| lock(&s.state).runs()).sum::<f64>();
        let file_reads = runs(g.files.iter().map(|(_, s)| s).collect());
        let repo_reads = runs(g.repos.iter().map(|(_, r)| &r.source).collect());
        health_of(
            "identities",
            "gatherer",
            &g.life,
            &[("files", g.files.len() as f64), ("fileReads", file_reads), ("repos", g.repos.len() as f64), ("repoReads", repo_reads)],
            Value::Null,
        )
    }
}
