//! The migration against fixture homes built in temp folders, the way agent-wiki's installers left
//! them (Rust 1.8.0 and the Node era, on macOS, Linux and Windows layouts), with a fake system in
//! place of launchctl, systemctl, sc, schtasks, reg, the apps' CLIs and the installer. Nothing here
//! runs a service manager or touches this machine's home.

use super::detect::{self, Ctx, Opts};
use super::journal::{Journal, crash};
use super::locate::{Layout, aw_dirs};
use super::sys::{Proc, Sys};
use super::{Mode, Out, execute};
use crate::install::files as xf;
use crate::install::sys::Ran;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xenolith_core::paths::Env;

const AW_PROTOCOL: &str = include_str!("testdata/agent-wiki-1.8.0-PROTOCOL.md");
const NOTE_TEXT: &str = "PRIVATE-NOTE-TEXT the dentist moved to Thursday";
const SECRET: &str = "sk-or-v1-PRIVATE-SECRET-0123456789";

// ---------------------------------------------------------------- the fake system

#[derive(Default)]
struct Win {
    service: Option<String>,
    task: bool,
    run_value: Option<String>,
}

struct FakeSys {
    os: String,
    env: Env,
    log: RefCell<Vec<String>>,
    procs: RefCell<Vec<Proc>>,
    clis: RefCell<Vec<&'static str>>,
    desktop: Cell<bool>,
    signed_in: Cell<bool>,
    port_busy: RefCell<Option<Value>>,
    win: RefCell<Win>,
    install_fails: Cell<bool>,
    /// A lock owner that lets go when agent-wiki is stopped (the tray's curator).
    release_on_stop: RefCell<Option<PathBuf>>,
}

impl FakeSys {
    fn new(os: &str, env: &Env) -> FakeSys {
        FakeSys {
            os: os.into(),
            env: env.clone(),
            log: RefCell::new(vec![]),
            procs: RefCell::new(vec![]),
            clis: RefCell::new(vec!["claude", "codex"]),
            desktop: Cell::new(false),
            signed_in: Cell::new(true),
            port_busy: RefCell::new(None),
            win: RefCell::new(Win::default()),
            install_fails: Cell::new(false),
            release_on_stop: RefCell::new(None),
        }
    }
    fn lay(&self) -> Layout {
        Layout::new(&self.env, &self.os)
    }
    fn kill(&self, needle: &str) {
        self.procs.borrow_mut().retain(|p| !p.command.contains(needle));
        if let Some(l) = self.release_on_stop.borrow_mut().take() {
            let _ = std::fs::remove_dir_all(l);
        }
    }
    fn ran(&self, needle: &str) -> bool {
        self.log.borrow().iter().any(|l| l.contains(needle))
    }
}

fn ran(ok: bool, stdout: &str) -> Ran {
    Ran { ok, code: Some(if ok { 0 } else { 1 }), stdout: stdout.into(), stderr: String::new() }
}

fn remove_toml_table(file: &Path, header: &str) {
    let _ = super::edits::edit(file, |t| {
        let mut out = vec![];
        let mut skip = false;
        for l in t.split('\n') {
            if l.trim_start().starts_with('[') {
                skip = l.trim() == header;
            }
            if !skip {
                out.push(l);
            }
        }
        Ok(Some(out.join("\n")))
    });
}

impl Sys for FakeSys {
    fn os(&self) -> &str {
        &self.os
    }
    fn run(&self, program: &Path, args: &[&str], env: &[(&str, &str)]) -> Ran {
        let name = program.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let line = format!("{name} {}{}", args.join(" "), env.iter().map(|(k, v)| format!(" [{k}={v}]")).collect::<String>());
        self.log.borrow_mut().push(line);
        let lay = self.lay();
        let a = |i: usize| args.get(i).copied().unwrap_or("");
        match name.as_str() {
            "launchctl" if a(0) == "bootout" => {
                let p = a(2);
                if p.contains("server") {
                    self.kill("--http");
                } else if p.contains("curator") {
                    self.kill(" curator");
                } else if p.contains("tray") {
                    self.kill("tray");
                }
                ran(true, "")
            }
            "systemctl" if args.contains(&"stop") || args.contains(&"disable") => {
                let u = args.last().copied().unwrap_or("");
                self.kill(if u.contains("curator") { " curator" } else { "--http" });
                ran(true, "")
            }
            "sc" => {
                let mut w = self.win.borrow_mut();
                match a(0) {
                    "query" => match &w.service {
                        Some(s) => ran(true, &format!("SERVICE_NAME: AgentWiki\n        STATE              : 4  {s}\n")),
                        None => ran(false, "The specified service does not exist as an installed service."),
                    },
                    "stop" => {
                        if w.service.is_some() {
                            w.service = Some("STOPPED".into());
                        }
                        drop(w);
                        self.kill("AgentWikiService");
                        self.kill("service --config");
                        ran(true, "")
                    }
                    "start" => {
                        w.service = w.service.as_ref().map(|_| "RUNNING".into());
                        ran(true, "")
                    }
                    "delete" => {
                        w.service = None;
                        ran(true, "")
                    }
                    _ => ran(true, ""),
                }
            }
            "schtasks" => {
                let mut w = self.win.borrow_mut();
                match a(0) {
                    "/query" => ran(w.task, ""),
                    "/delete" => {
                        w.task = false;
                        ran(true, "")
                    }
                    "/create" => {
                        w.task = true;
                        ran(true, "")
                    }
                    _ => ran(true, ""),
                }
            }
            "reg" => {
                let mut w = self.win.borrow_mut();
                match a(0) {
                    "query" => match &w.run_value {
                        Some(v) => ran(true, &format!("\n{}\n    AgentWikiTray    REG_SZ    {v}\n", detect::RUN_KEY)),
                        None => ran(false, ""),
                    },
                    "delete" => {
                        w.run_value = None;
                        ran(true, "")
                    }
                    "add" => {
                        let i = args.iter().position(|x| *x == "/d").map(|i| i + 1).unwrap_or(0);
                        w.run_value = args.get(i).map(|s| s.to_string());
                        ran(true, "")
                    }
                    _ => ran(true, ""),
                }
            }
            "claude" => {
                match (a(0), a(1)) {
                    ("plugin", "uninstall") => {
                        let _ = super::edits::edit(&lay.claude_settings, |t| {
                            let mut v: Value = serde_json::from_str(t).unwrap();
                            v["enabledPlugins"].as_object_mut().map(|o| o.remove(super::edits::CLAUDE_PLUGIN_ID));
                            Ok(Some(super::edits::to_json(&v)))
                        });
                        let f = lay.claude_dir.join("plugins").join("installed_plugins.json");
                        let _ = super::edits::edit(&f, |t| {
                            let mut v: Value = serde_json::from_str(t).unwrap();
                            v["plugins"].as_object_mut().map(|o| o.remove(super::edits::CLAUDE_PLUGIN_ID));
                            Ok(Some(super::edits::to_json(&v)))
                        });
                    }
                    ("plugin", "install") => {
                        let f = lay.claude_dir.join("plugins").join("installed_plugins.json");
                        let _ = super::edits::edit(&f, |t| {
                            let mut v: Value = serde_json::from_str(t).unwrap();
                            v["plugins"][a(2)] = json!([{ "scope": "user" }]);
                            Ok(Some(super::edits::to_json(&v)))
                        });
                    }
                    ("plugin", "marketplace") if a(2) == "add" => {
                        let f = lay.claude_dir.join("plugins").join("known_marketplaces.json");
                        let _ = super::edits::edit(&f, |t| {
                            let mut v: Value = serde_json::from_str(t).unwrap();
                            v[super::edits::CLAUDE_MARKET] = json!({ "source": "x" });
                            Ok(Some(super::edits::to_json(&v)))
                        });
                    }
                    ("plugin", "marketplace") if a(2) == "remove" => {
                        let f = lay.claude_dir.join("plugins").join("known_marketplaces.json");
                        let _ = super::edits::edit(&f, |t| {
                            let mut v: Value = serde_json::from_str(t).unwrap();
                            v.as_object_mut().map(|o| o.remove(super::edits::CLAUDE_MARKET));
                            Ok(Some(super::edits::to_json(&v)))
                        });
                    }
                    _ => {}
                }
                ran(true, "")
            }
            "codex" => match (a(0), a(1)) {
                ("login", "status") => ran(self.signed_in.get() && env.iter().any(|(k, v)| *k == "CODEX_HOME" && Path::new(v).join("auth.json").exists()), "Logged in"),
                ("plugin", "remove") => {
                    remove_toml_table(&lay.codex_config, &format!("[plugins.\"{}\"]", a(2)));
                    ran(true, "")
                }
                _ => ran(true, ""),
            },
            n if n.contains("tray") || n.contains("Tray") => {
                if args.contains(&"--quit") {
                    self.kill("tray");
                    self.kill("Tray");
                    self.kill(" curator");
                }
                ran(true, "")
            }
            _ => ran(true, ""),
        }
    }
    fn spawn(&self, program: &Path, args: &[&str]) {
        self.log.borrow_mut().push(format!("(start) {} {}", program.display(), args.join(" ")));
    }
    fn which(&self, name: &str) -> Option<PathBuf> {
        self.clis.borrow().contains(&name).then(|| PathBuf::from(name))
    }
    fn desktop_running(&self) -> bool {
        self.desktop.get()
    }
    fn processes(&self) -> Vec<Proc> {
        self.procs.borrow().clone()
    }
    fn port(&self, _port: u16) -> (bool, Option<Value>) {
        let b = self.port_busy.borrow();
        (b.is_some(), b.clone().filter(|v| !v.is_null()))
    }
    fn get_json(&self, _port: u16, path: &str) -> Option<Value> {
        let lay = self.lay();
        let cfg: Value = serde_json::from_str(&std::fs::read_to_string(lay.xen.config_file()).ok()?).ok()?;
        lay.xen.install_state().exists().then_some(())?;
        Some(if path == "/health" { json!({ "ok": true, "name": "xenolith" }) } else { json!({ "wikiDir": cfg["wikiDir"] }) })
    }
    fn mcp_start(&self, agent: &Path, _env: &[(String, String)]) -> Result<(bool, bool), String> {
        if !agent.exists() {
            return Err(format!("{} does not exist", agent.display()));
        }
        let cfg: Value = serde_json::from_str(&std::fs::read_to_string(self.lay().xen.config_file()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let ok = cfg["wikiDir"].as_str().is_some_and(|w| Path::new(w).join("PROTOCOL.md").exists());
        Ok((ok, ok))
    }
    fn xenolith(&self, args: &[String]) -> Ran {
        self.log.borrow_mut().push(format!("xenolith {}", args.join(" ")));
        match args.first().map(String::as_str) {
            Some("install") if self.install_fails.get() => ran(false, "Install failed: simulated"),
            Some("install") => sim_install(self, args),
            Some("uninstall") => sim_uninstall(self),
            _ => ran(false, "unknown"),
        }
    }
    fn uid(&self) -> u32 {
        501
    }
    fn sleep(&self, _ms: u64) {}
}

/// What `xenolith install` does to the files the migration watches (install/mod.rs), in short.
fn sim_install(sys: &FakeSys, args: &[String]) -> Ran {
    let lay = sys.lay();
    let opt = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let approve = !args.iter().any(|a| a == "--no-approve");
    let mut cfg: Value = std::fs::read_to_string(lay.xen.config_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}));
    let wiki = opt("--wiki-dir").map(PathBuf::from).or_else(|| cfg["wikiDir"].as_str().map(PathBuf::from)).unwrap_or_else(|| lay.xen_default_wiki.clone());
    cfg["wikiDir"] = json!(super::edits::fwd(&wiki));
    if args.iter().any(|a| a == super::retire::FLAG) {
        // The elevated step removes agent-wiki's service along with registering Xenolith's.
        sys.win.borrow_mut().service = None;
    }
    cfg["version"] = json!(xenolith_core::VERSION);
    if cfg["httpPort"].is_null() {
        cfg["httpPort"] = json!(opt("--port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(47850));
    }
    if cfg["curator"]["codexHome"].is_null() {
        cfg["curator"]["codexHome"] = json!(lay.xen.curator_codex_home().to_string_lossy());
    }
    std::fs::create_dir_all(cfg["curator"]["codexHome"].as_str().unwrap()).unwrap();
    xf::write_lf(&lay.xen.config_file(), &xf::to_json(&cfg)).unwrap();
    xenolith_core::wiki::ensure_wiki(&wiki).unwrap();
    xenolith_core::wiki::refresh_protocol(&wiki).unwrap();
    let agent = lay.xen.runtime_dir().join(lay.exe("xenolith"));
    std::fs::create_dir_all(agent.parent().unwrap()).unwrap();
    std::fs::write(&agent, "xenolith").unwrap();
    std::fs::create_dir_all(lay.xen.data_dir.join("marketplace")).unwrap();
    let r = (|| -> Result<(), crate::install::InstallError> {
        xf::edit_file(&lay.personal_market, |t| {
            let mut m: Value = t.map(|t| serde_json::from_str(t).unwrap()).unwrap_or_else(|| json!({ "name": "personal", "plugins": [] }));
            let list = m["plugins"].as_array_mut().unwrap();
            if !list.iter().any(|x| x["name"] == "xenolith") {
                list.push(json!({ "name": "xenolith" }));
            }
            Ok(Some(xf::to_json(&m)))
        })?;
        if !sys.desktop.get() {
            for d in &lay.desktop_configs {
                xf::edit_file(d, |t| {
                    let mut c: Value = t.map(|t| serde_json::from_str(t).unwrap()).unwrap_or_else(|| json!({}));
                    c["mcpServers"]["xenolith"] = json!({ "command": "xenolith", "args": ["serve"] });
                    Ok(Some(xf::to_json(&c)))
                })?;
            }
        }
        for f in [&lay.codex_agents, &lay.claude_md] {
            xf::edit_file(f, |t| Ok(Some(xf::upsert_block(t, xf::POINTER))))?;
        }
        if approve {
            xf::edit_file(&lay.codex_config, |t| xf::toml_set_approval(t, "xenolith@personal"))?;
            xf::edit_file(&lay.claude_settings, xf::claude_allow_set)?;
        }
        let mut state: Value = std::fs::read_to_string(lay.xen.install_state()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({ "created": [] }));
        state["wikiDir"] = cfg["wikiDir"].clone();
        state["version"] = json!(xenolith_core::VERSION);
        xf::write_lf(&lay.xen.install_state(), &xf::to_json(&state))?;
        Ok(())
    })();
    match r {
        Ok(()) => ran(true, "Installed."),
        Err(e) => ran(false, &format!("Install failed: {}", e.0)),
    }
}

fn sim_uninstall(sys: &FakeSys) -> Ran {
    let lay = sys.lay();
    let _ = xf::edit_file(&lay.personal_market, |t| {
        let Some(t) = t else { return Ok(None) };
        let mut m: Value = serde_json::from_str(t).unwrap();
        if let Some(l) = m["plugins"].as_array_mut() {
            l.retain(|x| x["name"] != "xenolith");
        }
        Ok(Some(xf::to_json(&m)))
    });
    for d in &lay.desktop_configs {
        let _ = xf::edit_file(d, |t| {
            let Some(t) = t else { return Ok(None) };
            let mut c: Value = serde_json::from_str(t).unwrap();
            if c["mcpServers"].as_object_mut().and_then(|o| o.remove("xenolith")).is_none() {
                return Ok(None);
            }
            Ok(Some(xf::to_json(&c)))
        });
    }
    for f in [&lay.codex_agents, &lay.claude_md] {
        let text = std::fs::read_to_string(f).ok();
        if let Some(next) = xf::remove_block(text.as_deref()) {
            let _ = xf::edit_file(f, |_| Ok(Some(next.clone())));
        }
    }
    let _ = xf::edit_file(&lay.codex_config, |t| xf::toml_remove_approval(t, "xenolith@personal"));
    let _ = xf::edit_file(&lay.claude_settings, xf::claude_allow_remove);
    let _ = std::fs::remove_dir_all(lay.xen.data_dir.join("marketplace"));
    ran(true, "Uninstalled.")
}

// ---------------------------------------------------------------- fixture homes

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    env: Env,
    os: String,
    era: &'static str,
    wiki: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn block(inner: &str) -> String {
    format!("<!-- AGENT-WIKI:START (managed by the agent-wiki installer; replaced on re-install) -->\n{inner}\n<!-- AGENT-WIKI:END -->")
}

static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Fixture {
    /// An empty home for `os`.
    fn home(os: &str) -> Fixture {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("xenolith-migrate-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let mut env: Env = Env::new();
        let h = home.to_string_lossy().into_owned();
        if os == "windows" {
            env.insert("USERPROFILE".into(), h.clone());
            env.insert("APPDATA".into(), format!("{h}/AppData/Roaming"));
            env.insert("LOCALAPPDATA".into(), format!("{h}/AppData/Local"));
        } else {
            env.insert("HOME".into(), h.clone());
        }
        Fixture { wiki: home.join("AgentWiki"), root, home, env, os: os.into(), era: "rust" }
    }

    fn lay(&self) -> Layout {
        Layout::new(&self.env, &self.os)
    }

    /// agent-wiki as its installer left it: `era` "rust" (1.8.0) or "node" (1.2, Windows tray in C#).
    fn agent_wiki(os: &str, era: &'static str) -> Fixture {
        let mut f = Fixture::home(os);
        f.era = era;
        f.install(false);
        f
    }

    fn exe(&self, n: &str) -> String {
        if self.os == "windows" { format!("{n}.exe") } else { n.into() }
    }

    fn install(&mut self, legacy: bool) {
        let lay = self.lay();
        let d = aw_dirs(&self.env, &self.os, &self.home, false);
        let (cfg_dir, data, state, logs) =
            if legacy { (lay.legacy.clone(), lay.legacy.clone(), lay.legacy.clone(), lay.legacy.join("logs")) } else { (d.config.clone(), d.data.clone(), d.state.clone(), d.log.clone()) };
        let version = if self.era == "rust" { "1.8.0" } else { "1.2.0" };
        let codex_home = data.join("curator").join("codex-home");
        write(
            &cfg_dir.join("config.json"),
            &xf::to_json(&json!({
                "wikiDir": self.wiki.to_string_lossy(), "version": version, "httpPort": 47821, "writeMode": "curated",
                "curator": { "model": "gpt-6.1-sol", "reasoningEffort": "high", "codexPath": "codex", "codexHome": codex_home.to_string_lossy() },
                "logs": { "retentionDays": 14 }, "ask": { "model": "gpt-6.1-sol-mini" }, "hooks": { "stop": true },
                "search": { "embeddings": { "enabled": true } },
            })),
        );
        let created: Vec<String> = vec![super::edits::fwd(&lay.codex_agents), super::edits::fwd(&lay.personal_market)];
        write(
            &state.join("install-state.json"),
            &xf::to_json(&json!({ "version": version, "wikiDir": self.wiki.to_string_lossy(), "codexPluginId": "agent-wiki@personal", "created": created, "transport": "http" })),
        );
        if self.era == "rust" {
            write(&data.join("runtime").join(self.exe("agent-wiki")), "agent-wiki program");
            write(&data.join("tray").join(self.exe("agent-wiki-tray")), "tray program");
            write(&data.join("tray").join("agent-wiki-tray.ini"), "agent=x\n");
        } else {
            write(&data.join("runtime").join("server.mjs"), "// server");
            write(&data.join("runtime").join("curator.mjs"), "// curator");
            if self.os == "windows" {
                write(&data.join("tray").join("AgentWikiTray.exe"), "C# tray");
                write(&data.join("tray").join("AgentWikiTray.ini"), "node=x\n");
            }
        }
        if self.os == "windows" {
            write(&data.join("tray").join("AgentWikiTray.task.xml"), "<Task/>");
            write(&data.join("service").join("AgentWikiService.ini"), "port=47821\n");
        }
        write(&data.join("marketplace").join(".claude-plugin").join("marketplace.json"), "{\"name\":\"agent-wiki-local\"}\n");
        write(&data.join("marketplace").join("plugins").join("agent-wiki").join(".mcp.json"), "{}\n");
        write(&codex_home.join("auth.json"), "{\"signed\":\"in\"}\n");
        write(&logs.join("requests-2026-10-01.jsonl"), "{\"tool\":\"wiki_log\",\"args\":\"clipped\"}\n");
        write(&data.join("paste-into-app-settings.md"), "pointer\n");

        // The wiki.
        let w = &self.wiki;
        write(&w.join("PROTOCOL.md"), AW_PROTOCOL);
        write(&w.join("index.md"), "# Index\n");
        write(&w.join("README.md"), "# Agent Wiki\n");
        for i in 0..5 {
            write(&w.join("pages").join(format!("page-{i}.md")), &format!("---\ntitle: Page {i}\n---\n{NOTE_TEXT} {i}\n"));
        }
        write(&w.join("log").join("2026").join("2026-10-01.md"), &format!("# 2026-10-01\n{NOTE_TEXT}\n"));
        write(&w.join("log").join("2026").join("2026-10-02.md"), "# 2026-10-02\n");
        for id in ["2026-10-20_10-00-00-000-aaaaaa", "2026-10-20_10-00-01-000-bbbbbb"] {
            write(&w.join("inbox").join(format!("{id}.md")), &format!("---\nid: {id}\n---\n{NOTE_TEXT}\n"));
        }
        write(&w.join(".curator").join("state").join("2026-10-20_10-00-01-000-bbbbbb.json"), "{\"attempts\":5,\"dead\":true}\n");
        write(&w.join(".curator").join("done").join("2026").join("10").join("2026-10-01_09-00-00-000-cccccc.md"), &format!("{NOTE_TEXT}\n"));
        write(&w.join(".curator").join("held").join("b1.json"), "{\"batch\":\"b1\"}\n");
        write(&w.join(".curator").join("vectors").join("google_gemini-embedding-2").join("ab").join("abcd.f32"), "1234");
        std::fs::create_dir_all(w.join(".locks")).unwrap();

        // The apps' files, as agent-wiki's installer edited them.
        write(&lay.claude_md, &format!("# My rules\n\nBe terse.\n\n{}\n", block("Agent Wiki pointer")));
        write(
            &lay.claude_settings,
            &xf::to_json(&json!({
                "env": { "OPENROUTER_API_KEY": SECRET },
                "permissions": { "allow": ["Bash(ls)", "mcp__plugin_agent-wiki_agent-wiki", "mcp__agent-wiki"] },
                "enabledPlugins": { "agent-wiki@agent-wiki-local": true, "other@x": true },
            })),
        );
        write(&lay.claude_dir.join("plugins").join("installed_plugins.json"), &xf::to_json(&json!({ "plugins": { "agent-wiki@agent-wiki-local": [{ "scope": "user" }] } })));
        write(&lay.claude_dir.join("plugins").join("known_marketplaces.json"), &xf::to_json(&json!({ "agent-wiki-local": { "source": "x" } })));
        write(&lay.codex_agents, &format!("{}\n", block("Agent Wiki pointer")));
        write(
            &lay.codex_config,
            &format!(
                "model = \"gpt-6.1-sol\"\n\n[plugins.\"agent-wiki@personal\"]\nenabled = true\n\n{}\n[plugins.\"agent-wiki@personal\".mcp_servers.agent-wiki]\ndefault_tools_approval_mode = \"approve\"\n",
                super::edits::TOML_COMMENT
            ),
        );
        write(
            &lay.personal_market,
            &xf::to_json(&json!({ "name": "personal", "interface": { "displayName": "Personal" }, "plugins": [{ "name": "agent-wiki", "source": { "source": "local", "path": "./x" } }] })),
        );
        let desktop_dir = match self.os.as_str() {
            "windows" => self.home.join("AppData").join("Roaming").join("Claude"),
            "macos" => self.home.join("Library").join("Application Support").join("Claude"),
            _ => self.home.join(".config").join("Claude"),
        };
        write(
            &desktop_dir.join("claude_desktop_config.json"),
            "{\r\n  \"mcpServers\": {\r\n    \"agent-wiki\": {\r\n      \"command\": \"x\"\r\n    },\r\n    \"other\": {\r\n      \"command\": \"y\"\r\n    }\r\n  }\r\n}\r\n",
        );
        let lay = self.lay();
        match self.os.as_str() {
            _ if self.era == "node" => {}
            "macos" => {
                for l in detect::AW_LABELS {
                    write(&lay.launch_agents.join(format!("{l}.plist")), &format!("<plist><string>{l}</string></plist>\n"));
                }
            }
            "linux" => {
                for u in detect::AW_UNITS {
                    write(&lay.systemd_user.join(u), &format!("[Service]\nExecStart={}\n", data.join("runtime").join("agent-wiki").display()));
                }
                write(&lay.autostart.join(detect::AW_AUTOSTART), "[Desktop Entry]\n");
            }
            _ => {}
        }
    }

    /// The fake system, with agent-wiki running as it would.
    fn sys(&self) -> FakeSys {
        let s = FakeSys::new(&self.os, &self.env);
        let lay = self.lay();
        let data = lay.data_candidates().into_iter().find(|d| d.join("runtime").exists()).unwrap_or_else(|| lay.standard.data.clone());
        let rt = data.join("runtime");
        let agent = rt.join(self.exe("agent-wiki"));
        let mut procs = vec![];
        let tray = data.join("tray").join(self.exe("agent-wiki-tray"));
        match (self.era, self.os.as_str()) {
            ("rust", "windows") => {
                // The service runs the server; the tray hosts the curator.
                procs.push(Proc { pid: 106, command: format!("{} service --config x.ini", agent.display()) });
                procs.push(Proc { pid: 103, command: format!("{} --from task", tray.display()) });
                procs.push(Proc { pid: 102, command: format!("{} curator --parent-stdin", agent.display()) });
            }
            ("rust", _) => {
                procs.push(Proc { pid: 101, command: format!("{} serve --http --port 47821 --exit-on-upgrade", agent.display()) });
                procs.push(Proc { pid: 102, command: format!("{} curator", agent.display()) });
                procs.push(Proc { pid: 103, command: format!("{} --from login", tray.display()) });
            }
            (_, "windows") => {
                procs.push(Proc { pid: 106, command: format!("{}", data.join("service").join("AgentWikiService.exe").display()) });
                procs.push(Proc { pid: 104, command: format!("{} --from task", data.join("tray").join("AgentWikiTray.exe").display()) });
                procs.push(Proc { pid: 105, command: format!("node {} curator", rt.join("curator.mjs").display()) });
            }
            _ => {}
        }
        if self.os == "windows" {
            *s.win.borrow_mut() = Win { service: Some("RUNNING".into()), task: true, run_value: Some(format!("\"{}\" --from run", data.join("tray").join("agent-wiki-tray.exe").display())) };
        }
        procs.push(Proc { pid: 200, command: "/usr/bin/vim notes.md".into() });
        *s.procs.borrow_mut() = procs;
        s
    }

    fn ctx<'a>(&self, sys: &'a FakeSys) -> Ctx<'a> {
        Ctx { sys, env: self.env.clone(), lay: self.lay(), opts: Opts { wiki_dir: None, port: None, sandbox: None, from: None }, wait_ms: 1000 }
    }
}

/// Every file under `dir` with its content hash (links as their target), sorted.
fn snapshot(dir: &Path) -> BTreeMap<String, String> {
    fn go(base: &Path, d: &Path, out: &mut BTreeMap<String, String>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            let t = e.file_type().unwrap();
            if t.is_symlink() {
                out.insert(rel, format!("-> {}", std::fs::read_link(&p).unwrap().display()));
            } else if t.is_dir() {
                out.insert(format!("{rel}/"), String::new());
                go(base, &p, out);
            } else {
                out.insert(rel, xenolith_core::text::sha256_hex(&std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    go(dir, dir, &mut out);
    out
}

fn run_mode(f: &Fixture, sys: &FakeSys, mode: Mode) -> (i32, Value, Vec<String>) {
    let mut ctx = f.ctx(sys);
    let out = Out::new(true);
    let (code, v) = execute(&mut ctx, &out, mode, false);
    (code, v, out.lines())
}

fn journal(f: &Fixture) -> Journal {
    Journal::load(&f.lay().work).unwrap().unwrap()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// What a finished migration must have left, whatever path it took there.
fn assert_migrated(f: &Fixture, sys: &FakeSys) {
    let lay = f.lay();
    let j = journal(f);
    assert_eq!(j.state, "done", "{:?}", j.steps);
    let cfg: Value = serde_json::from_str(&read(&lay.xen.config_file())).unwrap();
    assert_eq!(cfg["wikiDir"], json!(super::edits::fwd(&f.wiki)), "the wiki is adopted in place");
    assert_eq!(cfg["curator"]["reasoningEffort"], "high", "curator settings carried over");
    assert_eq!(cfg["ask"]["model"], "gpt-6.1-sol-mini");
    assert_eq!(cfg["hooks"]["stop"], true);
    assert_eq!(cfg["logs"]["retentionDays"], 14);
    assert_eq!(cfg["search"]["embeddings"]["credential"], detect::AW_CREDENTIAL, "the stored key keeps working");
    assert_eq!(cfg["httpPort"], 47850, "agent-wiki's port is not carried");
    assert!(PathBuf::from(cfg["curator"]["codexHome"].as_str().unwrap()).join("auth.json").exists(), "the curator stays signed in");
    assert!(!read(&lay.claude_md).contains("AGENT-WIKI") && read(&lay.claude_md).contains("XENOLITH:START") && read(&lay.claude_md).starts_with("# My rules"));
    assert!(!read(&lay.codex_agents).contains("AGENT-WIKI"));
    assert!(!read(&lay.claude_settings).contains("mcp__agent-wiki") && read(&lay.claude_settings).contains("mcp__xenolith") && read(&lay.claude_settings).contains(SECRET));
    assert!(!read(&lay.codex_config).contains("agent-wiki") && read(&lay.codex_config).contains("xenolith@personal"), "{}", read(&lay.codex_config));
    assert!(!read(&lay.personal_market).contains("agent-wiki"));
    for d in &lay.desktop_configs {
        let t = read(d);
        assert!(!t.contains("agent-wiki") && t.contains("\"other\"") && t.contains("\r\n"), "{t}");
    }
    assert!(!xenolith_core::inbox::is_paused(&f.wiki), "the curator runs again");
    let left = detect::registrations(&f.ctx(sys), &j.codex_plugin_id).0;
    assert!(left.is_empty(), "left: {:?}", left.iter().map(|r| format!("{} {}", r.kind, r.key)).collect::<Vec<_>>());
    assert_eq!(j.verify.as_ref().unwrap()["ok"], true, "{:?}", j.verify);
    assert_eq!(read(&f.wiki.join("PROTOCOL.md")), xenolith_core::wiki::DEFAULT_PROTOCOL.replace("\r\n", "\n"), "an unedited protocol becomes Xenolith's");
    assert!(detect::aw_processes(&f.ctx(sys), &sys.processes()).iter().all(|p| p.kind == "session"), "agent-wiki stopped");
}

// ---------------------------------------------------------------- layouts

#[test]
fn layouts_per_os_xdg_portable_and_captured_copies() {
    let f = Fixture::home("windows");
    let lay = f.lay();
    assert_eq!(lay.standard.config, f.home.join("AppData/Roaming").join("AgentWiki"));
    assert_eq!(lay.standard.data, f.home.join("AppData/Local").join("AgentWiki"));
    assert_eq!(lay.standard.log, f.home.join("AppData/Local").join("AgentWiki").join("logs"));
    // An app package's captured copy (MSIX LocalCache) is found too.
    let pkg = f.home.join("AppData/Local").join("Packages").join("Claude_pzs8sxrjxfjjc").join("LocalCache");
    std::fs::create_dir_all(pkg.join("Local").join("AgentWiki")).unwrap();
    std::fs::create_dir_all(pkg.join("Roaming").join("AgentWiki")).unwrap();
    std::fs::create_dir_all(pkg.join("Roaming").join("Claude")).unwrap();
    let lay = f.lay();
    assert_eq!(lay.captured.len(), 2);
    assert_eq!(lay.desktop_configs, vec![pkg.join("Roaming").join("Claude").join("claude_desktop_config.json")], "Store installs read the packaged copy");

    let m = Fixture::home("macos");
    let lay = m.lay();
    assert_eq!(lay.standard.data, m.home.join("Library/Application Support/Agent Wiki"));
    assert_eq!(lay.standard.log, m.home.join("Library/Logs/Agent Wiki"));
    assert_eq!(lay.standard.cache, m.home.join("Library/Caches/Agent Wiki"));

    let mut l = Fixture::home("linux");
    let xdg = l.root.join("xdg");
    l.env.insert("XDG_CONFIG_HOME".into(), xdg.join("cfg").to_string_lossy().into_owned());
    l.env.insert("XDG_STATE_HOME".into(), xdg.join("state").to_string_lossy().into_owned());
    l.env.insert("XDG_DATA_HOME".into(), "relative/ignored".into());
    let lay = l.lay();
    assert_eq!(lay.standard.config, xdg.join("cfg").join("agent-wiki"));
    assert_eq!(lay.standard.log, xdg.join("state").join("agent-wiki").join("logs"));
    assert_eq!(lay.standard.data, l.home.join(".local/share/agent-wiki"), "a relative XDG variable is ignored");
    assert_eq!(lay.systemd_user, xdg.join("cfg").join("systemd").join("user"));
    assert_eq!(lay.autostart, xdg.join("cfg").join("autostart"));

    // AGENT_WIKI_HOME: the programs used one folder; the installer still used the standard ones.
    let portable = l.root.join("portable");
    l.env.insert("AGENT_WIKI_HOME".into(), portable.to_string_lossy().into_owned());
    let lay = l.lay();
    assert_eq!(lay.overridden.as_ref().unwrap().config, portable);
    assert_eq!(lay.data_candidates()[0], portable, "the programs' own folder is looked at first");
    assert!(lay.data_candidates().contains(&lay.standard.data));
    assert_eq!(lay.aw_vars, vec!["AGENT_WIKI_HOME".to_string()]);
}

#[test]
fn finds_rust_and_node_installs_on_every_os() {
    for os in ["macos", "linux", "windows"] {
        for era in ["rust", "node"] {
            let f = Fixture::agent_wiki(os, era);
            let sys = f.sys();
            let found = detect::detect(&f.ctx(&sys));
            assert!(found.found && found.blockers.is_empty(), "{os} {era}: {:?}", found.blockers);
            assert_eq!(found.era, era, "{os}");
            assert_eq!(found.version.as_deref(), Some(if era == "rust" { "1.8.0" } else { "1.2.0" }));
            assert_eq!(found.wiki.source, "config.json");
            let s = found.wiki.scan.as_ref().unwrap();
            assert_eq!((s.pages.count, s.log_days.count, s.inbox.count, s.dead_letters, s.filed.count, s.held.count, s.vectors.count), (5, 2, 2, 1, 1, 1, 1));
            assert_eq!(found.wiki.protocol, "earlier-default");
            let kinds: Vec<&str> = found.registrations.iter().map(|r| r.kind.as_str()).collect();
            for k in ["Claude Code plugin", "Claude Code allow rules", "Claude Code marketplace", "instructions", "Claude desktop", "personal marketplace", "Codex plugin", "Codex approval"] {
                assert!(kinds.contains(&k), "{os} {era}: {k} missing from {kinds:?}");
            }
            match (os, era) {
                ("macos", "rust") => assert_eq!(kinds.iter().filter(|k| **k == "LaunchAgent").count(), 3),
                ("linux", "rust") => assert!(kinds.contains(&"user unit") && kinds.contains(&"autostart entry")),
                ("windows", _) => assert!(kinds.contains(&"Windows service") && kinds.contains(&"logon task") && kinds.contains(&"Run value")),
                _ => assert!(!kinds.contains(&"LaunchAgent") && !kinds.contains(&"user unit"), "the Node era had no services there"),
            }
            assert!(found.processes.iter().all(|p| p.pid != 200), "an editor on a note is not agent-wiki");
            assert_eq!(found.processes.is_empty(), era == "node" && os != "windows");
            assert_eq!(found.codex_home.action, "move");
            assert!(found.approve);
            let plan = detect::plan(&f.ctx(&sys), &found);
            assert_eq!(plan.len(), 7);
        }
    }
}

#[test]
fn pre_1_3_and_both_layouts_present() {
    let mut f = Fixture::home("linux");
    f.install(true);
    let sys = f.sys();
    let found = detect::detect(&f.ctx(&sys));
    assert!(found.pre13 && found.found, "{:?}", found.blockers);
    assert!(found.config_file.as_deref().unwrap().ends_with(".agent-wiki/config.json"));
    assert!(found.codex_home.from.as_deref().unwrap().contains(".agent-wiki/curator/codex-home"));
    // Both: a standard install that left logs behind in the old folder.
    let mut g = Fixture::home("macos");
    g.install(false);
    write(&g.lay().legacy.join("logs").join("service.log"), "old\n");
    let sys = g.sys();
    let found = detect::detect(&g.ctx(&sys));
    assert!(!found.pre13);
    assert!(found.folders.iter().any(|(k, _)| k == "before 1.3"));
    assert!(found.notes.iter().any(|n| n.contains(".agent-wiki")));
    run_mode(&g, &sys, Mode::Run);
    assert_migrated(&g, &sys);
    run_mode(&g, &sys, Mode::Finish);
    assert!(!g.lay().legacy.exists(), "--finish removes the pre-1.3 folder too");
}

// ---------------------------------------------------------------- the whole run, per OS and era

#[test]
fn a_run_moves_everything_and_a_second_run_is_a_no_op() {
    for os in ["macos", "linux", "windows"] {
        for era in ["rust", "node"] {
            let f = Fixture::agent_wiki(os, era);
            let sys = f.sys();
            let (code, v, lines) = run_mode(&f, &sys, Mode::Run);
            assert_eq!(code, 0, "{os} {era}: {v}\n{}", lines.join("\n"));
            assert_migrated(&f, &sys);
            match (os, era) {
                ("macos", "rust") => assert!(sys.ran("launchctl bootout gui/501") && detect::AW_LABELS.iter().all(|l| !f.lay().launch_agents.join(format!("{l}.plist")).exists())),
                ("linux", "rust") => assert!(sys.ran("systemctl --user disable --now agent-wiki.service") && !f.lay().autostart.join(detect::AW_AUTOSTART).exists()),
                (_, "node") if os != "windows" => assert!(!sys.ran("launchctl") && !sys.ran("systemctl")),
                _ => {
                    assert!(sys.ran("sc stop AgentWiki"));
                    let install = sys.log.borrow().iter().find(|l| l.starts_with("xenolith install")).cloned().unwrap();
                    assert!(install.contains("--retire-agent-wiki-service") && install.contains("--retire-grant"), "{install}");
                    assert!(sys.ran("schtasks /delete /tn AgentWikiTray") && sys.ran("reg delete"));
                }
            }
            if era == "node" {
                assert!(sys.ran("codex app-server daemon stop"), "{os}: the Node-era daemon is stopped");
                assert!(sys.log.borrow().iter().any(|l| l.contains("app-server") && l.contains("CODEX_HOME=") && l.contains("codex-home")), "with its own Codex home");
            }
            assert!(sys.ran("claude plugin uninstall agent-wiki@agent-wiki-local") && sys.ran("codex plugin remove agent-wiki@personal"));
            // agent-wiki's programs stay until --finish, so a rollback can use them.
            assert!(f.lay().data_candidates().iter().any(|d| d.join("runtime").exists()));
            let before = snapshot(&f.home);
            let (code, _, lines) = run_mode(&f, &sys, Mode::Run);
            assert_eq!(code, 0);
            assert!(lines.iter().any(|l| l.contains("already moved")), "{lines:?}");
            assert_eq!(snapshot(&f.home), before, "{os} {era}: a second run changes nothing");
            let (code, _, _) = run_mode(&f, &sys, Mode::Resume);
            assert_eq!(code, 0);
            assert_eq!(snapshot(&f.home), before, "nor does --resume with nothing waiting");
        }
    }
}

#[test]
fn a_dry_run_writes_nothing() {
    for os in ["macos", "linux", "windows"] {
        let f = Fixture::agent_wiki(os, "rust");
        let sys = f.sys();
        let before = snapshot(&f.root);
        let (code, v, _) = run_mode(&f, &sys, Mode::DryRun);
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["ready"], true);
        assert_eq!(snapshot(&f.root), before, "{os}");
        assert!(sys.log.borrow().iter().all(|l| l.starts_with("sc query") || l.starts_with("schtasks /query") || l.starts_with("reg query")), "{:?}", sys.log.borrow());
        // Its data names every registration with its file and key.
        let regs = v["found"]["registrations"].as_array().unwrap();
        assert!(regs.iter().any(|r| r["kind"] == "Claude Code allow rules" && r["file"].as_str().unwrap().ends_with(".claude/settings.json")));
    }
}

#[test]
fn nothing_installed_is_nothing_to_do() {
    let f = Fixture::home("macos");
    let sys = FakeSys::new("macos", &f.env);
    let (code, v, _) = run_mode(&f, &sys, Mode::DryRun);
    assert_eq!((code, v["found"]["found"].clone()), (0, json!(false)));
    let (code, v, _) = run_mode(&f, &sys, Mode::Run);
    assert_eq!(code, 1);
    assert!(v["error"].as_str().unwrap().contains("no agent-wiki install"));
    assert!(!f.lay().work.exists());
}

// ---------------------------------------------------------------- kills, resume, rollback

/// How many journal writes a full run makes for this fixture.
fn writes_in_a_run(os: &str, era: &'static str) -> usize {
    let f = Fixture::agent_wiki(os, era);
    let sys = f.sys();
    crash::arm(None);
    run_mode(&f, &sys, Mode::Run);
    let n = crash::seen();
    assert_migrated(&f, &sys);
    n
}

/// Runs `f` for each of `range` on a few threads (each kill point has its own fixture home, and the
/// kill switch is per thread), so every point is covered without a slow test.
fn each_in_parallel(range: std::ops::RangeInclusive<usize>, f: impl Fn(usize) + Sync) {
    let points: Vec<usize> = range.collect();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    std::thread::scope(|s| {
        for t in 0..threads {
            let (points, f) = (&points, &f);
            s.spawn(move || points.iter().skip(t).step_by(threads).for_each(|at| f(*at)));
        }
    });
}

/// Kills the run at each of its journal writes in turn; --resume then finishes it.
fn kill_at_every_step_then_resume(os: &str, era: &'static str) {
    let total = writes_in_a_run(os, era);
    assert!(total > 15, "{total}");
    each_in_parallel(1..=total, |at| {
        let f = Fixture::agent_wiki(os, era);
        let sys = f.sys();
        crash::arm(Some(at));
        let (code, v, _) = run_mode(&f, &sys, Mode::Run);
        crash::arm(None);
        assert_eq!(code, 1, "{os}: killed at {at}");
        assert!(v["error"].as_str().unwrap().contains(crash::KILLED), "{v}");
        if !f.lay().work.join("journal.json").exists() {
            // Killed before the first journal write: nothing happened; start again.
            assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
        } else {
            // A plain --run refuses to start over a migration in progress.
            let (code, v, _) = run_mode(&f, &sys, Mode::Run);
            assert!(code == 1 && v["error"].as_str().unwrap().contains("--resume"), "{os} at {at}: {v}");
            let (code, v, lines) = run_mode(&f, &sys, Mode::Resume);
            assert_eq!(code, 0, "{os}: resumed after a kill at write {at}: {v}\n{}", lines.join("\n"));
        }
        assert_migrated(&f, &sys);
    });
}

#[test]
fn a_kill_at_every_step_then_resume_ends_migrated_macos() {
    kill_at_every_step_then_resume("macos", "rust");
}

#[test]
fn a_kill_at_every_step_then_resume_ends_migrated_linux() {
    kill_at_every_step_then_resume("linux", "rust");
}

#[test]
fn a_kill_at_every_step_then_resume_ends_migrated_windows_node_era() {
    kill_at_every_step_then_resume("windows", "node");
}

/// The home as it was, except what a rollback leaves on purpose: Xenolith's own folders (its
/// programs stay, as `xenolith uninstall` leaves them), the migration's journal, and in the wiki
/// the generated index and the copy of the replaced protocol in .history/.
fn comparable(f: &Fixture, snap: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let lay = f.lay();
    let skip: Vec<String> = [&lay.xen.config_dir, &lay.xen.data_dir, &lay.xen.state_dir, &lay.xen.log_dir, &lay.xen.cache_dir]
        .iter()
        .map(|d| d.strip_prefix(&f.home).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    snap.into_iter()
        .filter(|(k, _)| {
            !skip.iter().any(|s| k.starts_with(s.as_str()))
                && !k.starts_with("AgentWiki/.history")
                && !k.starts_with("AgentWiki/index.md")
                && !k.starts_with("AgentWiki/.gitattributes")
                && !k.starts_with("AgentWiki/.gitignore")
        })
        .collect()
}

#[test]
fn rollback_after_a_full_run_puts_everything_back() {
    for (os, era) in [("macos", "rust"), ("linux", "rust"), ("windows", "rust"), ("windows", "node")] {
        let f = Fixture::agent_wiki(os, era);
        let sys = f.sys();
        let before = comparable(&f, snapshot(&f.home));
        assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
        let (code, v, lines) = run_mode(&f, &sys, Mode::Rollback);
        assert_eq!(code, 0, "{os}: {v}\n{}", lines.join("\n"));
        assert_eq!(comparable(&f, snapshot(&f.home)), before, "{os} {era}: the home is as it was");
        assert!(!f.lay().xen.config_file().exists(), "Xenolith's config is gone (it did not exist before)");
        assert!(sys.ran("xenolith uninstall"));
        assert!(sys.ran("claude plugin install agent-wiki@agent-wiki-local") && sys.ran("codex plugin add agent-wiki@personal"));
        match os {
            "macos" => assert!(sys.ran("launchctl bootstrap gui/501")),
            "linux" => assert!(sys.ran("systemctl --user enable --now agent-wiki.service")),
            _ => {
                let w = sys.win.borrow();
                assert!(w.task && w.run_value.is_some(), "the tray's start at sign-in is back");
                drop(w);
                // The service itself needs an elevated step, and the report says which.
                assert!(journal(&f).pending.iter().any(|p| p.contains("Run as administrator")));
            }
        }
        assert_eq!(journal(&f).state, "rolled-back");
        let (code, v, _) = run_mode(&f, &sys, Mode::Rollback);
        assert_eq!(code, 0, "{v}");
        let (code, v, _) = run_mode(&f, &sys, Mode::Resume);
        assert!(code == 1 && v["error"].as_str().unwrap().contains("--run starts a new one"), "{v}");
        if os == "windows" {
            // The person put the service back as the report said.
            sys.win.borrow_mut().service = Some("RUNNING".into());
        }
        // Moving again later starts a new migration, with the old journal kept beside it.
        let (code, v, lines) = run_mode(&f, &sys, Mode::Run);
        assert_eq!(code, 0, "{os}: {v}\n{}", lines.join("\n"));
        assert_migrated(&f, &sys);
        assert!(std::fs::read_dir(f.lay().work).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("rolled-back-")));
    }
}

#[test]
fn rollback_after_a_kill_at_every_step_puts_everything_back() {
    let (os, era) = ("macos", "rust");
    let total = writes_in_a_run(os, era);
    each_in_parallel(2..=total, |at| {
        let f = Fixture::agent_wiki(os, era);
        let sys = f.sys();
        let before = comparable(&f, snapshot(&f.home));
        crash::arm(Some(at));
        run_mode(&f, &sys, Mode::Run);
        crash::arm(None);
        let (code, v, lines) = run_mode(&f, &sys, Mode::Rollback);
        assert_eq!(code, 0, "killed at {at}: {v}\n{}", lines.join("\n"));
        let after = comparable(&f, snapshot(&f.home));
        let diff: Vec<_> = before.iter().filter(|(k, v)| after.get(*k) != Some(v)).map(|(k, _)| k.clone()).chain(after.keys().filter(|k| !before.contains_key(*k)).cloned()).collect();
        assert!(diff.is_empty(), "killed at {at}, then rolled back: differs in {diff:?}");
    });
}

#[test]
fn a_xenolith_installed_before_keeps_its_settings_through_a_rollback() {
    let f = Fixture::agent_wiki("linux", "rust");
    let sys = f.sys();
    // Xenolith was installed first, with an empty wiki of its own (no pages, so no blocker).
    assert!(sim_install(&sys, &["install".to_string()]).ok);
    let lay = f.lay();
    let xen_cfg = read(&lay.xen.config_file());
    let found = detect::detect(&f.ctx(&sys));
    assert!(found.xenolith.installed && found.blockers.is_empty(), "{:?}", found.blockers);
    let before = snapshot(&f.home);
    assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
    assert_migrated(&f, &sys);
    let (code, v, lines) = run_mode(&f, &sys, Mode::Rollback);
    assert_eq!(code, 0, "{v}\n{}", lines.join("\n"));
    assert!(!sys.ran("xenolith uninstall"), "Xenolith stays installed");
    assert_eq!(read(&lay.xen.config_file()), xen_cfg, "with its own settings");
    for f2 in [&lay.claude_md, &lay.claude_settings, &lay.codex_config, &lay.personal_market] {
        let rel = f2.strip_prefix(&f.home).unwrap().to_string_lossy().replace('\\', "/");
        assert_eq!(snapshot(&f.home).get(&rel), before.get(&rel), "{rel} as it was, with both apps' entries");
    }
    // An uninstalled Xenolith counts as not installed.
    let g = Fixture::agent_wiki("linux", "rust");
    let sys = g.sys();
    write(&g.lay().xen.install_state(), "{\"installedAt\":\"2026-10-01T10:00:00\",\"uninstalledAt\":\"2026-10-02T10:00:00\"}\n");
    assert!(!detect::detect(&g.ctx(&sys)).xenolith.installed);
}

#[test]
fn rollback_leaves_a_file_you_changed_since() {
    let f = Fixture::agent_wiki("macos", "rust");
    let sys = f.sys();
    assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
    let lay = f.lay();
    let mine = format!("{}\nMy own line after the move.\n", read(&lay.claude_md));
    write(&lay.claude_md, &mine);
    let (code, v, lines) = run_mode(&f, &sys, Mode::Rollback);
    assert_eq!(code, 1, "{v}");
    let now = read(&lay.claude_md);
    assert!(now.contains("My own line after the move.") && !now.contains("AGENT-WIKI"), "your edit is kept, and agent-wiki's block is not forced back in: {now}");
    assert!(!now.contains("XENOLITH:START"), "Xenolith's uninstall still takes its own block out");
    assert!(lines.iter().any(|l| l.contains("CLAUDE.md") && l.contains("changed since the migration") && l.contains("backup/")), "{lines:?}");
    assert!(read(&lay.claude_settings).contains("mcp__agent-wiki"), "the other files are back");
}

#[test]
fn resume_finishes_what_waited_for_claude_desktop() {
    let f = Fixture::agent_wiki("macos", "rust");
    let sys = f.sys();
    sys.desktop.set(true);
    let (code, v, lines) = run_mode(&f, &sys, Mode::DryRun);
    assert_eq!(code, 0);
    assert!(v["found"]["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("Claude desktop is running")));
    let desktop = f.lay().desktop_configs[0].clone();
    let before = read(&desktop);
    let (code, _, lines2) = run_mode(&f, &sys, Mode::Run);
    assert_eq!(code, 0, "{}", lines2.join("\n"));
    assert_eq!(read(&desktop), before, "left alone while it runs");
    let (code, v, _) = run_mode(&f, &sys, Mode::Finish);
    assert!(code == 1 && v["error"].as_str().unwrap().contains("quit Claude desktop"), "--finish waits for it too: {v}");
    let j = journal(&f);
    assert!(j.pending.iter().any(|p| p.contains("quit Claude desktop")));
    assert!(lines2.iter().any(|l| l.contains("quit Claude desktop")), "the report says what to do: {lines:?}");
    sys.desktop.set(false);
    let (code, _, _) = run_mode(&f, &sys, Mode::Resume);
    assert_eq!(code, 0);
    let t = read(&desktop);
    assert!(!t.contains("agent-wiki") && t.contains("xenolith"), "{t}");
    assert!(journal(&f).pending.is_empty());
}

#[test]
fn a_failed_install_stops_with_the_reason_and_resumes() {
    let f = Fixture::agent_wiki("linux", "rust");
    let sys = f.sys();
    sys.install_fails.set(true);
    let (code, v, _) = run_mode(&f, &sys, Mode::Run);
    assert_eq!(code, 1);
    let e = v["error"].as_str().unwrap();
    assert!(e.contains("step 5") && e.contains("simulated") && e.contains("--resume") && e.contains("--rollback"), "{e}");
    assert!(read(&f.lay().claude_md).contains("AGENT-WIKI"), "agent-wiki's registrations stay until Xenolith is installed");
    sys.install_fails.set(false);
    assert_eq!(run_mode(&f, &sys, Mode::Resume).0, 0);
    assert_migrated(&f, &sys);
}

// ---------------------------------------------------------------- the wiki

#[test]
fn a_wiki_path_with_spaces_and_a_missing_folder() {
    let mut f = Fixture::home("macos");
    f.wiki = f.home.join("My Documents").join("Agent Wiki Notes");
    f.install(false);
    let sys = f.sys();
    assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
    assert_migrated(&f, &sys);

    let mut h = Fixture::home("macos");
    h.install(false);
    std::fs::remove_dir_all(&h.wiki).unwrap();
    let sys = h.sys();
    let found = detect::detect(&h.ctx(&sys));
    assert!(found.blockers.iter().any(|b| b.contains("does not exist") && b.contains("--wiki-dir")), "{:?}", found.blockers);
}

/// A wiki reached through a link is adopted through the link; its files stay where they are.
#[cfg(unix)]
#[test]
fn a_linked_wiki_is_adopted_through_the_link() {
    let mut g = Fixture::home("linux");
    let real = g.root.join("elsewhere").join("wiki");
    g.wiki = real.clone();
    g.install(false);
    let link = g.home.join("AgentWiki");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let cfg = g.lay().standard.config.join("config.json");
    let mut v: Value = serde_json::from_str(&read(&cfg)).unwrap();
    v["wikiDir"] = json!(link.to_string_lossy());
    write(&cfg, &xf::to_json(&v));
    g.wiki = link.clone();
    let sys = g.sys();
    let found = detect::detect(&g.ctx(&sys));
    assert!(found.wiki.link.is_some() && found.blockers.is_empty(), "{:?}", found.blockers);
    assert_eq!(run_mode(&g, &sys, Mode::Run).0, 0);
    assert_migrated(&g, &sys);
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    // --move-wiki does not move a link.
    let (code, v) = {
        let mut ctx = g.ctx(&sys);
        let out = Out::new(true);
        execute(&mut ctx, &out, Mode::Finish, true)
    };
    assert_eq!(code, 1);
    assert!(v["error"].as_str().unwrap().contains("is a link"), "{v}");
}

#[test]
fn protocol_unedited_lf_and_crlf_is_replaced_an_edited_one_kept() {
    for crlf in [false, true] {
        let f = Fixture::agent_wiki("linux", "rust");
        let text = if crlf { AW_PROTOCOL.replace('\n', "\r\n") } else { AW_PROTOCOL.to_string() };
        write(&f.wiki.join("PROTOCOL.md"), &text);
        let sys = f.sys();
        assert_eq!(detect::detect(&f.ctx(&sys)).wiki.protocol, "earlier-default", "crlf {crlf}");
        assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
        assert_migrated(&f, &sys);
        assert!(std::fs::read_dir(f.wiki.join(".history")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("PROTOCOL-")), "the old one is kept in .history/");
        // Rollback restores the old protocol while it is still Xenolith's default.
        assert_eq!(run_mode(&f, &sys, Mode::Rollback).0, 0);
        assert_eq!(read(&f.wiki.join("PROTOCOL.md")), text);
    }
    let f = Fixture::agent_wiki("linux", "rust");
    let edited = format!("{AW_PROTOCOL}\n## Mine\n\nAlways call wiki_start first.\n");
    write(&f.wiki.join("PROTOCOL.md"), &edited);
    let sys = f.sys();
    let found = detect::detect(&f.ctx(&sys));
    assert!(found.wiki.protocol == "edited" && found.wiki.protocol_old_names);
    let (code, _, lines) = run_mode(&f, &sys, Mode::Run);
    assert_eq!(code, 0);
    assert_eq!(read(&f.wiki.join("PROTOCOL.md")), edited, "your edit is kept");
    assert!(lines.iter().any(|l| l.contains("--update-protocol")), "the report offers the rename");
    let (code, v, _) = run_mode(&f, &sys, Mode::UpdateProtocol);
    assert_eq!(code, 0, "{v}");
    let now = read(&f.wiki.join("PROTOCOL.md"));
    assert!(now.contains("Always call context_start first.") && !now.contains("wiki_start") && now.contains("## Mine"));
    assert!(std::fs::read_dir(f.wiki.join(".history")).unwrap().flatten().any(|e| read(&e.path()) == edited), "the old text is in .history/");
    let (_, v, _) = run_mode(&f, &sys, Mode::UpdateProtocol);
    assert_eq!(v["changed"], false, "a second time has nothing to rename");
}

#[test]
fn blockers_stop_the_run_before_anything_changes() {
    type Setup = fn(&Fixture, &FakeSys);
    let cases: Vec<(&str, Setup)> = vec![
        ("spaces", |f, _| std::fs::create_dir_all(f.wiki.join("spaces").join("mine")).unwrap()),
        ("not valid JSON", |f, _| write(&f.lay().claude_settings, "{ \"permissions\": ")),
        ("not valid TOML", |f, _| write(&f.lay().codex_config, "model = = \"x\"\n")),
        ("not a JSON object", |f, _| write(&f.lay().personal_market, "[]")),
        ("Xenolith already has a wiki", |f, _| write(&f.home.join("Xenolith").join("pages").join("mine.md"), "# Mine\n")),
        ("port 47850 is in use", |_, s| *s.port_busy.borrow_mut() = Some(Value::Null)),
        ("AGENT_WIKI_DIR names", |_, _| {}),
    ];
    for (want, setup) in cases {
        let mut f = Fixture::agent_wiki("macos", "rust");
        if want.starts_with("AGENT_WIKI_DIR") {
            f.env.insert("AGENT_WIKI_DIR".into(), f.root.join("other").to_string_lossy().into_owned());
        }
        let sys = f.sys();
        setup(&f, &sys);
        let before = snapshot(&f.root);
        let (code, v, _) = run_mode(&f, &sys, Mode::Run);
        assert_eq!(code, 1, "{want}");
        assert!(v["error"].as_str().unwrap().contains(want), "{want}: {v}");
        assert_eq!(snapshot(&f.root), before, "{want}: nothing changed");
    }
    // Xenolith's own service on the port is no blocker.
    let f = Fixture::agent_wiki("macos", "rust");
    let sys = f.sys();
    *sys.port_busy.borrow_mut() = Some(json!({ "ok": true, "name": "xenolith" }));
    assert!(detect::detect(&f.ctx(&sys)).blockers.is_empty());
    // --wiki-dir settles AGENT_WIKI_DIR against config.json.
    let mut g = Fixture::agent_wiki("macos", "rust");
    g.env.insert("AGENT_WIKI_DIR".into(), g.root.join("other").to_string_lossy().into_owned());
    let sys = g.sys();
    let mut ctx = g.ctx(&sys);
    ctx.opts.wiki_dir = Some(g.wiki.clone());
    let found = detect::detect(&ctx);
    assert!(found.blockers.is_empty() && found.warnings.iter().any(|w| w.contains("AGENT_WIKI_DIR")), "{:?}", found.blockers);
}

#[test]
fn a_backlog_held_changes_and_vectors_survive_and_the_curator_waits() {
    let f = Fixture::agent_wiki("macos", "rust");
    for i in 0..40 {
        let id = format!("2026-10-21_10-00-{:02}-000-dddddd", i);
        write(&f.wiki.join("inbox").join(format!("{id}.md")), &format!("---\nid: {id}\n---\nnote {i}\n"));
    }
    write(&f.wiki.join(".curator").join("held").join("b2.json"), "{}\n");
    write(&f.wiki.join(".curator").join("held").join("done").join("b0.json"), "{}\n");
    let sys = f.sys();
    let before = super::scan::scan(&f.wiki);
    assert_eq!((before.inbox.count, before.held.count), (42, 2));
    assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
    assert_eq!(super::scan::scan(&f.wiki), before, "every note, held change and vector is where it was");
    assert!(sys.ran("launchctl bootout gui/501") && journal(&f).steps["2"].status == "done");
}

#[test]
fn stale_locks_are_ignored_and_live_ones_waited_for() {
    let f = Fixture::agent_wiki("linux", "rust");
    let locks = f.wiki.join(".locks");
    write(&locks.join("write.lock").join("owner"), "{\"pid\":2147483000,\"start\":1}");
    let sys = f.sys();
    assert!(detect::detect(&f.ctx(&sys)).wiki.locks.iter().all(|l| !l.live), "a dead owner's lock is stale");
    // This process holds the curator lock: live. It lets go when agent-wiki stops (here: the tray quits).
    write(&locks.join("curator.lock").join("owner"), &format!("{{\"pid\":{}}}", std::process::id()));
    *sys.release_on_stop.borrow_mut() = Some(locks.join("curator.lock"));
    let found = detect::detect(&f.ctx(&sys));
    assert!(found.wiki.locks.iter().any(|l| l.name == "curator" && l.live));
    assert!(found.warnings.iter().any(|w| w.contains("curator lock")));
    assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
    // One that is never let go stops step 2 with who holds it.
    let g = Fixture::agent_wiki("linux", "rust");
    write(&g.wiki.join(".locks").join("write.lock").join("owner"), &format!("{{\"pid\":{},\"label\":\"an editor\"}}", std::process::id()));
    let sys = g.sys();
    let (code, v, _) = run_mode(&g, &sys, Mode::Run);
    assert_eq!(code, 1);
    let e = v["error"].as_str().unwrap();
    assert!(e.contains("step 2") && e.contains("write lock") && e.contains("an editor"), "{e}");
    std::fs::remove_dir_all(g.wiki.join(".locks").join("write.lock")).unwrap();
    assert_eq!(run_mode(&g, &sys, Mode::Resume).0, 0);
    assert_migrated(&g, &sys);
}

// ---------------------------------------------------------------- config

#[test]
fn config_custom_port_credentials_and_an_outside_codex_home() {
    let f = Fixture::agent_wiki("macos", "rust");
    let cfg_file = f.lay().standard.config.join("config.json");
    let outside = f.root.join("my-codex");
    write(&outside.join("auth.json"), "{}");
    let mut v: Value = serde_json::from_str(&read(&cfg_file)).unwrap();
    v["httpPort"] = json!(48000);
    v["curator"]["codexHome"] = json!(outside.to_string_lossy());
    v["search"]["embeddings"]["credential"] = json!("Mine/OpenRouter");
    v["somethingElse"] = json!(1);
    write(&cfg_file, &xf::to_json(&v));
    let sys = f.sys();
    let found = detect::detect(&f.ctx(&sys));
    assert!(found.dropped.iter().any(|d| d.starts_with("httpPort 48000")), "{:?}", found.dropped);
    assert!(found.dropped.iter().any(|d| d.starts_with("somethingElse")));
    assert_eq!(found.codex_home.action, "kept");
    assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
    let x: Value = serde_json::from_str(&read(&f.lay().xen.config_file())).unwrap();
    assert_eq!(x["httpPort"], 47850);
    assert_eq!(x["search"]["embeddings"]["credential"], "Mine/OpenRouter", "a credential you named stays");
    assert_eq!(x["curator"]["codexHome"], json!(super::edits::fwd(&outside)), "kept, written with / as wikiDir is");
    assert!(outside.join("auth.json").exists() && x.get("somethingElse").is_none());
    // Embeddings off: no credential is added.
    let (c, _) = detect::carry(&json!({ "search": { "embeddings": { "enabled": false } } }), &json!({}), Path::new("/w"), "/h");
    assert!(c["search"]["embeddings"].get("credential").is_none());
    // --port goes to the install.
    let g = Fixture::agent_wiki("linux", "rust");
    let sys = g.sys();
    let mut ctx = g.ctx(&sys);
    ctx.opts.port = Some(47999);
    let out = Out::new(true);
    assert_eq!(execute(&mut ctx, &out, Mode::Run, false).0, 0);
    assert!(sys.ran("--port 47999"));
}

// ---------------------------------------------------------------- the apps' files

#[test]
fn bom_crlf_created_files_and_missing_clis() {
    let f = Fixture::agent_wiki("windows", "rust");
    let lay = f.lay();
    let settings = format!("\u{feff}{}", read(&lay.claude_settings).replace('\n', "\r\n"));
    write(&lay.claude_settings, &settings);
    // agent-wiki created AGENTS.md with only its block: after the move it holds Xenolith's, and
    // Xenolith's uninstall knows it may delete it once it is empty.
    let sys = f.sys();
    sys.clis.borrow_mut().clear();
    let found = detect::detect(&f.ctx(&sys));
    assert!(found.warnings.iter().any(|w| w.contains("claude CLI")) && found.warnings.iter().any(|w| w.contains("codex CLI")), "{:?}", found.warnings);
    let (code, _, lines) = run_mode(&f, &sys, Mode::Run);
    assert_eq!(code, 0, "{}", lines.join("\n"));
    let s = read(&lay.claude_settings);
    assert!(s.starts_with('\u{feff}') && s.contains("\r\n") && !s.contains("mcp__agent-wiki") && s.contains("mcp__xenolith"), "{s:?}");
    let state: Value = serde_json::from_str(&read(&lay.xen.install_state())).unwrap();
    assert!(state["created"].as_array().unwrap().iter().any(|c| c.as_str().unwrap().ends_with(".codex/AGENTS.md")), "{state}");
    let j = journal(&f);
    assert!(j.pending.iter().any(|p| p.contains("Claude Code plugin")) && j.pending.iter().any(|p| p.contains("Codex plugin")), "{:?}", j.pending);
    assert!(lines.iter().any(|l| l.contains("/plugin")), "the report says how");
}

// ---------------------------------------------------------------- finish

#[test]
fn finish_waits_for_agent_wiki_to_stop_then_deletes_its_folders_and_can_move_the_wiki() {
    let f = Fixture::agent_wiki("macos", "rust");
    let sys = f.sys();
    // A Claude Code session still runs agent-wiki's server.
    let agent = f.lay().standard.data.join("runtime").join("agent-wiki");
    sys.procs.borrow_mut().push(Proc { pid: 300, command: format!("{} serve", agent.display()) });
    let (code, _, lines) = run_mode(&f, &sys, Mode::Run);
    assert_eq!(code, 0, "{}", lines.join("\n"));
    assert!(journal(&f).warnings.iter().any(|w| w.contains("restart Claude Code")));
    let (code, v, _) = run_mode(&f, &sys, Mode::Finish);
    assert_eq!(code, 1);
    assert!(v["error"].as_str().unwrap().contains("pid 300 (session)"), "{v}");
    sys.kill("agent-wiki");
    let lay = f.lay();
    let mut ctx = f.ctx(&sys);
    let out = Out::new(true);
    let (code, v) = execute(&mut ctx, &out, Mode::Finish, true);
    assert_eq!(code, 0, "{v}\n{:?}", out.lines());
    for d in [&lay.standard.config, &lay.standard.data, &lay.standard.log, &lay.standard.cache] {
        assert!(!d.exists(), "{} is gone", d.display());
    }
    assert!(!f.wiki.exists() && lay.xen_default_wiki.join("PROTOCOL.md").exists(), "the wiki moved to ~/Xenolith");
    assert_eq!(super::scan::scan(&lay.xen_default_wiki), journal(&f).snapshot.unwrap(), "with everything in it");
    let cfg: Value = serde_json::from_str(&read(&lay.xen.config_file())).unwrap();
    assert_eq!(cfg["wikiDir"], json!(super::edits::fwd(&lay.xen_default_wiki)));
    assert!(sys.ran(&format!("xenolith install --wiki-dir {}", super::edits::fwd(&lay.xen_default_wiki))));
    assert!(read(&lay.claude_settings).contains(SECRET), "nothing else is touched");
    assert_eq!(journal(&f).state, "finished");
    let (code, v, _) = run_mode(&f, &sys, Mode::Rollback);
    assert!(code == 1 && v["error"].as_str().unwrap().contains("--finish already removed"), "{v}");
}

#[test]
fn finish_on_windows_waits_for_the_service_and_never_deletes_a_folder_holding_the_wiki() {
    let mut f = Fixture::home("windows");
    // A portable install with the wiki inside its folder (AGENT_WIKI_HOME).
    let portable = f.home.join("AgentWiki");
    f.env.insert("AGENT_WIKI_HOME".into(), portable.to_string_lossy().into_owned());
    f.wiki = portable.join("wiki");
    f.install(false);
    let sys = f.sys();
    assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
    // The UAC prompt was declined in this story: the service is still registered.
    sys.win.borrow_mut().service = Some("STOPPED".into());
    sys.procs.borrow_mut().clear();
    let (code, v, _) = run_mode(&f, &sys, Mode::Finish);
    assert!(code == 1 && v["error"].as_str().unwrap().contains("AgentWiki service is still registered"), "{v}");
    sys.win.borrow_mut().service = None;
    let (code, v, lines) = run_mode(&f, &sys, Mode::Finish);
    assert_eq!(code, 0, "{v}");
    assert!(f.wiki.join("PROTOCOL.md").exists(), "the wiki is never deleted");
    assert!(lines.iter().any(|l| l.contains("left in place") && l.contains("is inside it")), "{lines:?}");
    assert!(!f.lay().standard.data.exists());
}

// ---------------------------------------------------------------- privacy

#[test]
fn the_journal_and_log_hold_no_note_text_or_secrets() {
    let f = Fixture::agent_wiki("linux", "rust");
    let sys = f.sys();
    assert_eq!(run_mode(&f, &sys, Mode::Run).0, 0);
    assert_eq!(run_mode(&f, &sys, Mode::Rollback).0, 0);
    let work = f.lay().work;
    for name in ["journal.json", "migrate.log"] {
        let t = read(&work.join(name));
        assert!(!t.is_empty(), "{name}");
        for bad in [NOTE_TEXT, SECRET, "dentist", "gpt-6.1-sol-mini"] {
            assert!(!t.contains(bad), "{name} contains {bad}");
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for e in std::fs::read_dir(work.join("backup")).unwrap().flatten() {
            assert_eq!(e.metadata().unwrap().permissions().mode() & 0o077, 0, "{} is yours alone", e.path().display());
        }
        assert_eq!(std::fs::metadata(work.join("journal.json")).unwrap().permissions().mode() & 0o077, 0);
    }
}

// ---------------------------------------------------------------- the command line

#[test]
fn arguments() {
    let a = |s: &str| super::parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>());
    assert_eq!(a("agent-wiki").unwrap().mode, Mode::DryRun);
    assert_eq!(a("agent-wiki --run --json").unwrap().mode, Mode::Run);
    assert!(a("agent-wiki --run --rollback").is_err());
    assert!(a("agent-wiki --move-wiki").is_err());
    assert!(a("agent-wiki --port x").is_err());
    assert!(a("something-else").is_err());
    assert!(a("agent-wiki --bogus").is_err());
    let f = a("agent-wiki --finish --move-wiki --wiki-dir /w --port 47900").unwrap();
    assert!(f.move_wiki && f.opts.port == Some(47900) && f.opts.wiki_dir == Some(xenolith_core::wiki::absolute(Path::new("/w"))));
}

// ---------------------------------------------------------------- size

/// 10k pages, 50k filed notes and 100k vectors: the dry run and the run each stay under a minute.
/// Run with `cargo test -- --ignored migrate`.
#[test]
#[ignore]
fn a_large_wiki_migrates_in_under_a_minute() {
    let f = Fixture::agent_wiki("macos", "rust");
    let t0 = std::time::Instant::now();
    let filler = "x".repeat(400);
    for i in 0..10_000 {
        std::fs::write(f.wiki.join("pages").join(format!("p-{i}.md")), format!("# Page {i}\n{filler}\n")).unwrap();
    }
    for m in 0..50 {
        let d = f.wiki.join(".curator").join("done").join("2026").join(format!("{m:02}"));
        std::fs::create_dir_all(&d).unwrap();
        for i in 0..1000 {
            std::fs::write(d.join(format!("n-{i}.md")), format!("note {m} {i}\n{filler}\n")).unwrap();
        }
    }
    for a in 0..256 {
        let d = f.wiki.join(".curator").join("vectors").join("m").join(format!("{a:02x}"));
        std::fs::create_dir_all(&d).unwrap();
        for i in 0..391 {
            std::fs::write(d.join(format!("{a:02x}{i:030}.f32")), [0u8; 3072]).unwrap();
        }
    }
    eprintln!("fixture: {:?}", t0.elapsed());
    let sys = f.sys();
    let t = std::time::Instant::now();
    let (code, v, _) = run_mode(&f, &sys, Mode::DryRun);
    let dry = t.elapsed();
    assert_eq!(code, 0);
    assert_eq!(v["found"]["wiki"]["scan"]["vectors"]["count"], 100_097);
    let t = std::time::Instant::now();
    let (code, _, _) = run_mode(&f, &sys, Mode::Run);
    let run = t.elapsed();
    assert_eq!(code, 0);
    eprintln!("dry run {dry:?}, run {run:?}");
    assert!(dry.as_secs() < 60 && run.as_secs() < 60);
}
