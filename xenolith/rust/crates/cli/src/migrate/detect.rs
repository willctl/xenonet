//! The dry run: finds an agent-wiki install (Node or Rust era, 1.3+ folders or the pre-1.3
//! ~/.agent-wiki), its wiki and what the wiki holds, every registration it made in the apps, the
//! processes it runs, and what would stop the migration. It only reads: files, the process list,
//! the port, and read-only queries of the Windows service manager.

use super::edits::{self, fwd};
use super::locate::Layout;
use super::scan::{Scan, scan};
use super::sys::{Proc, Sys};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use xenolith_core::paths::Env;

pub const AW_SERVICE: &str = "AgentWiki";
pub const AW_TASK: &str = "AgentWikiTray";
pub const AW_RUN_VALUE: &str = "AgentWikiTray";
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
/// agent-wiki's LaunchAgents (macOS) and user units and autostart entry (Linux).
pub const AW_LABELS: [&str; 3] = ["com.agentwiki.server", "com.agentwiki.curator", "com.agentwiki.tray"];
pub const AW_UNITS: [&str; 2] = ["agent-wiki.service", "agent-wiki-curator.service"];
pub const AW_AUTOSTART: &str = "agent-wiki-tray.desktop";
pub const AW_PORT: u64 = 47821;
pub const XEN_PORT: u16 = 47850;
/// The credential name agent-wiki read the OpenRouter key under; Xenolith's default is another.
pub const AW_CREDENTIAL: &str = "AgentWiki/OpenRouter";

pub struct Opts {
    pub wiki_dir: Option<PathBuf>,
    pub port: Option<u16>,
    pub sandbox: Option<PathBuf>,
    pub from: Option<PathBuf>,
}

/// What every part of the migration works from.
pub struct Ctx<'a> {
    pub sys: &'a dyn Sys,
    pub env: Env,
    pub lay: Layout,
    pub opts: Opts,
    /// How long to wait for agent-wiki's locks and service to let go (tests wait less).
    pub wait_ms: u64,
}

impl Ctx<'_> {
    pub fn port(&self) -> u16 {
        self.opts.port.unwrap_or(XEN_PORT)
    }
    pub fn sandbox(&self) -> bool {
        self.opts.sandbox.is_some()
    }
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Reg {
    pub kind: String,
    pub file: Option<String>,
    pub key: String,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AwProc {
    pub pid: i64,
    /// tray | server | curator | service | session (a server an app started) | other
    pub kind: String,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LockInfo {
    pub name: String,
    pub live: bool,
    pub owner: String,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WikiInfo {
    pub path: Option<String>,
    pub source: String,
    pub exists: bool,
    pub link: Option<String>,
    pub scan: Option<Scan>,
    /// missing | earlier-default | current | edited
    pub protocol: String,
    pub protocol_hash: Option<String>,
    pub protocol_old_names: bool,
    pub spaces_dir: bool,
    pub locks: Vec<LockInfo>,
    pub paused: bool,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Programs {
    /// (program, its settings file): agent-wiki-tray and the earlier C# AgentWikiTray.
    pub trays: Vec<(String, Option<String>)>,
    pub agent: Option<String>,
    pub server_mjs: Option<String>,
    pub market: Option<String>,
    pub task_xml: Option<String>,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct XenInfo {
    pub installed: bool,
    pub config_file: String,
    pub has_config: bool,
    pub wiki: Option<String>,
    pub wiki_pages: usize,
    pub codex_home: String,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexHomePlan {
    pub from: Option<String>,
    pub to: String,
    /// move | kept (custom, or Xenolith's own is signed in) | absent
    pub action: String,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub found: bool,
    pub era: String,
    pub version: Option<String>,
    pub pre13: bool,
    pub uninstalled_at: Option<String>,
    pub config_file: Option<String>,
    #[serde(skip)]
    pub config: Value,
    #[serde(skip)]
    pub install_state: Value,
    pub folders: Vec<(String, String)>,
    pub programs: Programs,
    pub wiki: WikiInfo,
    pub codex_home: CodexHomePlan,
    pub codex_plugin_id: String,
    pub registrations: Vec<Reg>,
    pub processes: Vec<AwProc>,
    pub xenolith: XenInfo,
    /// The Xenolith config.json the migration writes (the settings carried over).
    pub carry: Value,
    pub dropped: Vec<String>,
    pub approve: bool,
    pub port: u16,
    pub port_busy: bool,
    pub desktop_running: bool,
    pub claude_cli: bool,
    pub codex_cli: bool,
    /// Windows: the AgentWiki service's state, the logon task, the Run value.
    pub aw_service: Option<String>,
    pub aw_task: bool,
    pub aw_run_value: Option<String>,
    pub journal: Option<String>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

fn read_json_soft(file: &Path) -> Result<Option<Value>, String> {
    match edits::read(file)? {
        Some(t) => edits::parse_json(&t, file).map(Some),
        None => Ok(None),
    }
}

/// Windows: `sc query AgentWiki`'s state, or None (not installed).
pub fn win_service(sys: &dyn Sys) -> Option<String> {
    let r = sys.run(Path::new("sc"), &["query", AW_SERVICE], &[]);
    if !r.ok {
        return None;
    }
    static STATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"STATE\s*:\s*[0-9]+\s+(\w+)").unwrap());
    Some(STATE.captures(&r.stdout).map(|c| c[1].to_string()).unwrap_or_else(|| "UNKNOWN".into()))
}

pub fn win_task(sys: &dyn Sys) -> bool {
    sys.run(Path::new("schtasks"), &["/query", "/tn", AW_TASK], &[]).ok
}

pub fn win_run_value(sys: &dyn Sys) -> Option<String> {
    let r = sys.run(Path::new("reg"), &["query", RUN_KEY, "/v", AW_RUN_VALUE], &[]);
    if !r.ok {
        return None;
    }
    r.stdout.lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string())).filter(|v| !v.is_empty())
}

/// Whether a command line is one of agent-wiki's programs, and which.
pub fn classify(cmd: &str, program_dirs: &[PathBuf], windows: bool) -> Option<String> {
    static NAMED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)(^|[/\\ "'])(agent-wiki(\.exe)?["']?\s+(serve|hook|curator|service)\b|agent-wiki-tray|AgentWikiTray|AgentWikiService)"#).unwrap());
    let norm = |s: &str| if windows { s.replace('\\', "/").to_lowercase() } else { s.to_string() };
    let c = norm(cmd);
    let under = program_dirs.iter().any(|d| {
        let d = norm(&d.to_string_lossy());
        !d.is_empty() && c.contains(&format!("{}/", d.trim_end_matches('/')))
    });
    if !under && !NAMED.is_match(cmd) {
        return None;
    }
    let l = cmd.to_lowercase();
    Some(
        if l.contains("agent-wiki-tray") || l.contains("agentwikitray") {
            "tray"
        } else if l.contains("agentwikiservice") || l.contains(" service --config") {
            "service"
        } else if l.contains("--http") {
            "server"
        } else if l.contains(" curator") || l.contains("curator.mjs") {
            "curator"
        } else if l.contains(" serve") || l.contains("server.mjs") {
            "session"
        } else {
            "other"
        }
        .to_string(),
    )
}

pub fn aw_processes(ctx: &Ctx, procs: &[Proc]) -> Vec<AwProc> {
    let dirs = ctx.lay.data_candidates();
    procs.iter().filter_map(|p| classify(&p.command, &dirs, ctx.lay.os == "windows").map(|kind| AwProc { pid: p.pid, kind })).collect()
}

/// A lock of the wiki and whether its owner still runs (read-only: no lock is broken here).
fn lock(wiki: &Path, name: &str) -> Option<LockInfo> {
    let dir = wiki.join(".locks").join(format!("{name}.lock"));
    if !dir.is_dir() {
        return None;
    }
    let owner = std::fs::read_to_string(dir.join("owner")).ok().and_then(|t| xenolith_core::lock::parse_owner(&t));
    let live = match &owner {
        // An owner that has not written itself down yet is about to: count it as live.
        None => true,
        Some(o) if cfg!(windows) => xenolith_core::lock::process_start_ms(o.pid).is_some(),
        Some(o) => xenolith_core::lock::owner_liveness(&wiki.join(".locks"), o) != xenolith_core::lock::Liveness::Dead,
    };
    let label = owner.map(|o| format!("pid {}{}", o.pid, o.label.map(|l| format!(" ({l})")).unwrap_or_default())).unwrap_or_else(|| "unknown".into());
    Some(LockInfo { name: name.into(), live, owner: label })
}

pub fn locks(wiki: &Path) -> Vec<LockInfo> {
    ["curator", "write"].iter().filter_map(|n| lock(wiki, n)).collect()
}

static OLD_TOOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bwiki_(start|search|read|log|upsert_page)\b").unwrap());

pub fn protocol_state(wiki: &Path) -> (String, Option<String>, bool) {
    let Some(text) = std::fs::read_to_string(wiki.join("PROTOCOL.md")).ok() else { return ("missing".into(), None, false) };
    let h = xenolith_core::text::hash_text(&text, 16);
    let state = if h == xenolith_core::text::hash_text(xenolith_core::wiki::DEFAULT_PROTOCOL, 16) {
        "current"
    } else if xenolith_core::wiki::OLD_PROTOCOLS.contains(&h.as_str()) {
        "earlier-default"
    } else {
        "edited"
    };
    (state.into(), Some(h), OLD_TOOL.is_match(&text))
}

/// The files the migration (or the install it runs) may change, with what each is: backed up in
/// step 1, restored by --rollback.
pub fn tracked_files(lay: &Layout, wiki: Option<&Path>) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = vec![
        (lay.claude_md.clone(), "Claude Code instructions".into()),
        (lay.claude_settings.clone(), "Claude Code settings".into()),
        (lay.codex_agents.clone(), "Codex instructions".into()),
        (lay.codex_config.clone(), "Codex config".into()),
        (lay.personal_market.clone(), "personal plugin marketplace".into()),
    ];
    out.extend(lay.desktop_configs.iter().map(|d| (d.clone(), "Claude desktop config".to_string())));
    out.push((lay.xen.config_file(), "Xenolith config".into()));
    out.push((lay.xen.install_state(), "Xenolith install state".into()));
    if let Some(w) = wiki {
        out.push((w.join("PROTOCOL.md"), "the wiki's PROTOCOL.md".into()));
    }
    match lay.os.as_str() {
        "macos" => {
            for l in AW_LABELS {
                let f = lay.launch_agents.join(format!("{l}.plist"));
                if f.exists() {
                    out.push((f, format!("LaunchAgent {l}")));
                }
            }
        }
        "windows" => {}
        _ => {
            for u in AW_UNITS {
                let f = lay.systemd_user.join(u);
                if f.exists() {
                    out.push((f, format!("user unit {u}")));
                }
            }
            let f = lay.autostart.join(AW_AUTOSTART);
            if f.exists() {
                out.push((f, "autostart entry".into()));
            }
        }
    }
    out
}

/// agent-wiki's registrations in the apps and the OS, found from files (and, on Windows, read-only
/// queries). Also returns the files that could not be read, for the blockers.
pub fn registrations(ctx: &Ctx, codex_id: &str) -> (Vec<Reg>, Vec<String>) {
    let lay = &ctx.lay;
    let mut regs = vec![];
    let mut bad = vec![];
    let mut add = |kind: &str, file: Option<&Path>, key: &str| regs.push(Reg { kind: kind.into(), file: file.map(fwd), key: key.into() });
    match read_json_soft(&lay.claude_settings) {
        Ok(Some(v)) => {
            if edits::claude_plugin_enabled(&v) {
                add("Claude Code plugin", Some(&lay.claude_settings), &format!("enabledPlugins.{}", edits::CLAUDE_PLUGIN_ID));
            }
            let allow = edits::allow_entries(&v);
            if !allow.is_empty() {
                add("Claude Code allow rules", Some(&lay.claude_settings), &format!("permissions.allow: {}", allow.join(", ")));
            }
        }
        Ok(None) => {}
        Err(e) => bad.push(e),
    }
    for (f, key) in [("installed_plugins.json", edits::CLAUDE_PLUGIN_ID), ("known_marketplaces.json", edits::CLAUDE_MARKET)] {
        let file = lay.claude_dir.join("plugins").join(f);
        if let Ok(Some(v)) = read_json_soft(&file) {
            if edits::json_mentions(&v, key) {
                add(if f.starts_with("known") { "Claude Code marketplace" } else { "Claude Code plugin" }, Some(&file), key);
            }
        }
    }
    for f in [&lay.claude_md, &lay.codex_agents] {
        match edits::read(f) {
            Ok(Some(t)) if edits::has_block(&t.replace("\r\n", "\n")) => add("instructions", Some(f), "AGENT-WIKI block"),
            Err(e) => bad.push(e),
            _ => {}
        }
    }
    for f in &lay.desktop_configs {
        match read_json_soft(f) {
            Ok(Some(v)) if edits::desktop_has(&v) => add("Claude desktop", Some(f), "mcpServers.agent-wiki"),
            Err(e) => bad.push(e),
            _ => {}
        }
    }
    match read_json_soft(&lay.personal_market) {
        Ok(Some(v)) if edits::market_has(&v) => add("personal marketplace", Some(&lay.personal_market), "plugins[agent-wiki]"),
        Err(e) => bad.push(e),
        _ => {}
    }
    match edits::read(&lay.codex_config) {
        Ok(Some(t)) => match edits::parse_toml(&t, &lay.codex_config) {
            Ok(tb) => {
                let mut ids = edits::toml_plugin_ids(&tb);
                if !ids.contains(&codex_id.to_string()) {
                    ids.push(codex_id.to_string());
                }
                for id in ids {
                    if edits::toml_has_plugin(&tb, &id) {
                        add("Codex plugin", Some(&lay.codex_config), &format!("plugins.\"{id}\""));
                    }
                    if edits::toml_has_approval(&tb, &id) {
                        add("Codex approval", Some(&lay.codex_config), &format!("plugins.\"{id}\".mcp_servers.agent-wiki"));
                    }
                }
            }
            Err(e) => bad.push(e),
        },
        Ok(None) => {}
        Err(e) => bad.push(e),
    }
    match lay.os.as_str() {
        "macos" => {
            for l in AW_LABELS {
                let f = lay.launch_agents.join(format!("{l}.plist"));
                if f.exists() {
                    add("LaunchAgent", Some(&f), l);
                }
            }
        }
        "windows" => {
            if let Some(s) = win_service(ctx.sys) {
                add("Windows service", None, &format!("{AW_SERVICE} ({s})"));
            }
            if win_task(ctx.sys) {
                add("logon task", None, &format!("\\{AW_TASK}"));
            }
            if win_run_value(ctx.sys).is_some() {
                add("Run value", None, &format!("{RUN_KEY}\\{AW_RUN_VALUE}"));
            }
        }
        _ => {
            for u in AW_UNITS {
                let f = lay.systemd_user.join(u);
                if f.exists() {
                    add("user unit", Some(&f), u);
                }
            }
            let f = lay.autostart.join(AW_AUTOSTART);
            if f.exists() {
                add("autostart entry", Some(&f), AW_AUTOSTART);
            }
        }
    }
    (regs, bad)
}

/// Whether `p` is inside (or is) `dir`.
pub fn inside(p: &Path, dir: &Path) -> bool {
    p.starts_with(dir)
}

/// The Xenolith config.json the migration writes: Xenolith's own (if any) with agent-wiki's
/// settings over it, and the wiki adopted in place. Returns it and what was left behind.
pub fn carry(aw: &Value, xen_prev: &Value, wiki: &Path, codex_home: &str) -> (Value, Vec<String>) {
    let mut out = xen_prev.as_object().cloned().unwrap_or_default();
    let mut dropped = vec![];
    const KEEP: [&str; 6] = ["writeMode", "curator", "ask", "logs", "hooks", "search"];
    for (k, v) in aw.as_object().into_iter().flatten() {
        if KEEP.contains(&k.as_str()) {
            match (out.get_mut(k), v) {
                (Some(Value::Object(mine)), Value::Object(theirs)) => {
                    for (kk, vv) in theirs {
                        mine.insert(kk.clone(), vv.clone());
                    }
                }
                _ => {
                    out.insert(k.clone(), v.clone());
                }
            }
        } else if k == "httpPort" {
            if v.as_u64() != Some(AW_PORT) {
                dropped.push(format!("httpPort {v}: Xenolith serves on its own port ({XEN_PORT}), or the one you pass with --port"));
            }
        } else if !["version", "wikiDir"].contains(&k.as_str()) {
            dropped.push(format!("{k}: not a Xenolith setting; it stays in agent-wiki's config.json"));
        }
    }
    out.insert("wikiDir".into(), json!(fwd(wiki)));
    let cur = out.entry("curator").or_insert_with(|| json!({}));
    if !cur.is_object() {
        *cur = json!({});
    }
    cur["codexHome"] = json!(codex_home);
    if let Some(e) = out.get_mut("search").and_then(|s| s.get_mut("embeddings")).filter(|e| e["enabled"] == true) {
        if e["credential"].as_str().is_none_or(str::is_empty) {
            // agent-wiki read the key under its own name; Xenolith's default name would miss it.
            e["credential"] = json!(AW_CREDENTIAL);
        }
    }
    (Value::Object(out), dropped)
}

/// Finds everything. Never writes.
pub fn detect(ctx: &Ctx) -> Found {
    detect_with(ctx, true)
}

/// `scan`: count and fingerprint the wiki too (a large wiki takes seconds; the steps that only need
/// to know what is registered and running skip it).
pub fn detect_with(ctx: &Ctx, scan_wiki: bool) -> Found {
    let lay = &ctx.lay;
    let mut f = Found { era: "unknown".into(), port: ctx.port(), approve: true, ..Default::default() };
    f.folders = lay.aw_folders().into_iter().map(|(k, p)| (k, fwd(&p))).collect();

    // config.json and install-state.json: the most recent layout that has them.
    let config_file = lay.config_candidates().into_iter().find(|c| c.exists());
    if let Some(c) = &config_file {
        f.config_file = Some(fwd(c));
        match read_json_soft(c) {
            Ok(Some(v)) => f.config = v,
            Ok(None) => {}
            Err(e) => f.blockers.push(format!("{e}: fix it (or restore its backup) so its settings can be carried over")),
        }
        f.pre13 = c.starts_with(&lay.legacy);
    }
    if f.config.is_null() {
        f.config = json!({});
    }
    f.install_state = lay.state_candidates().into_iter().find_map(|s| read_json_soft(&s).ok().flatten()).unwrap_or_else(|| json!({}));
    f.version = f.config["version"].as_str().or(f.install_state["version"].as_str()).map(String::from);
    f.uninstalled_at = f.install_state["uninstalledAt"].as_str().map(String::from);
    f.codex_plugin_id = f.install_state["codexPluginId"].as_str().unwrap_or(edits::DEFAULT_CODEX_ID).to_string();

    // The programs: in whichever of agent-wiki's folders holds them.
    for d in lay.data_candidates() {
        let agent = d.join("runtime").join(lay.exe("agent-wiki"));
        if agent.exists() && f.programs.agent.is_none() {
            f.programs.agent = Some(fwd(&agent));
        }
        let mjs = d.join("runtime").join("server.mjs");
        if mjs.exists() && f.programs.server_mjs.is_none() {
            f.programs.server_mjs = Some(fwd(&mjs));
        }
        for (exe, ini) in [(lay.exe("agent-wiki-tray"), "agent-wiki-tray.ini"), ("AgentWikiTray.exe".to_string(), "AgentWikiTray.ini")] {
            let t = d.join("tray").join(&exe);
            if t.exists() {
                let i = d.join("tray").join(ini);
                f.programs.trays.push((fwd(&t), i.exists().then(|| fwd(&i))));
            }
        }
        let m = d.join("marketplace");
        if m.is_dir() && f.programs.market.is_none() {
            f.programs.market = Some(fwd(&m));
        }
        let x = d.join("tray").join("AgentWikiTray.task.xml");
        if x.exists() && f.programs.task_xml.is_none() {
            f.programs.task_xml = Some(fwd(&x));
        }
    }
    f.era = if f.programs.agent.is_some() {
        "rust"
    } else if f.programs.server_mjs.is_some() {
        "node"
    } else {
        "unknown"
    }
    .into();

    // The wiki: --wiki-dir, then AGENT_WIKI_DIR, then config.json's wikiDir, then ~/AgentWiki.
    let cfg_wiki = f.config["wikiDir"].as_str().or(f.install_state["wikiDir"].as_str()).filter(|s| !s.trim().is_empty()).map(PathBuf::from);
    let env_wiki = ctx.env.get("AGENT_WIKI_DIR").filter(|v| !v.is_empty()).map(PathBuf::from);
    let (wiki, source) = if let Some(w) = &ctx.opts.wiki_dir {
        (w.clone(), "--wiki-dir")
    } else if let Some(w) = &env_wiki {
        if cfg_wiki.as_ref().is_some_and(|c| c != w) {
            f.blockers.push(format!(
                "AGENT_WIKI_DIR names {} but agent-wiki's config.json names {}: the apps started from this shell used one and the service the other. Pass --wiki-dir with the one to keep.",
                fwd(w),
                fwd(cfg_wiki.as_ref().unwrap())
            ));
        }
        (w.clone(), "AGENT_WIKI_DIR")
    } else if let Some(w) = &cfg_wiki {
        (w.clone(), "config.json")
    } else {
        (lay.home.join("AgentWiki"), "default")
    };
    let wiki = xenolith_core::wiki::absolute(&wiki);
    f.wiki.path = Some(fwd(&wiki));
    f.wiki.source = source.into();
    f.wiki.exists = wiki.is_dir();
    f.wiki.link = std::fs::read_link(&wiki).ok().map(|t| fwd(&t));
    f.found = config_file.is_some() || !f.folders.is_empty() || f.wiki.exists && wiki.join("PROTOCOL.md").exists();
    if f.wiki.exists {
        f.wiki.scan = scan_wiki.then(|| scan(&wiki));
        let (p, h, old) = protocol_state(&wiki);
        f.wiki.protocol = p;
        f.wiki.protocol_hash = h;
        f.wiki.protocol_old_names = old;
        f.wiki.spaces_dir = wiki.join(xenolith_core::spaces::SPACES_DIR).exists();
        f.wiki.locks = locks(&wiki);
        f.wiki.paused = xenolith_core::inbox::is_paused(&wiki);
    } else if wiki.exists() {
        f.blockers.push(format!("{} is not a folder", fwd(&wiki)));
    }

    // Xenolith as it is now.
    let xen_cfg_file = lay.xen.config_file();
    f.xenolith.config_file = fwd(&xen_cfg_file);
    // Installed, and not uninstalled since (an uninstall leaves its install-state.json behind).
    f.xenolith.installed = read_json_soft(&lay.xen.install_state()).ok().flatten().is_some_and(|s| {
        let at = |k: &str| s[k].as_str().unwrap_or("").to_string();
        s["uninstalledAt"].is_null() || at("installedAt") > at("uninstalledAt")
    });
    f.xenolith.codex_home = fwd(&lay.xen.curator_codex_home());
    let xen_cfg = match read_json_soft(&xen_cfg_file) {
        Ok(v) => {
            f.xenolith.has_config = v.is_some();
            v.unwrap_or_else(|| json!({}))
        }
        Err(e) => {
            f.blockers.push(format!("{e}: fix it or move it aside"));
            json!({})
        }
    };
    let xen_wiki = xen_cfg["wikiDir"].as_str().map(PathBuf::from).unwrap_or_else(|| lay.xen_default_wiki.clone());
    if xen_wiki.is_dir() && xenolith_core::wiki::absolute(&xen_wiki) != wiki {
        let pages = std::fs::read_dir(xen_wiki.join("pages")).map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".md")).count()).unwrap_or(0);
        let notes = std::fs::read_dir(xen_wiki.join("inbox")).map(|rd| rd.flatten().count()).unwrap_or(0);
        f.xenolith.wiki = Some(fwd(&xen_wiki));
        f.xenolith.wiki_pages = pages;
        if pages + notes > 0 {
            f.blockers.push(format!(
                "Xenolith already has a wiki with {pages} pages at {}. Adopting agent-wiki's folder would set it aside. Move its pages into {} (or move it away), then run again.",
                fwd(&xen_wiki),
                fwd(&wiki)
            ));
        }
    }

    // The curator's sign-in.
    let aw_codex = f.config["curator"]["codexHome"].as_str().map(PathBuf::from).or_else(|| lay.data_candidates().into_iter().map(|d| d.join("curator").join("codex-home")).find(|p| p.is_dir()));
    let xen_codex = lay.xen.curator_codex_home();
    let ours = |p: &Path| lay.data_candidates().iter().chain(lay.aw_folders().iter().map(|(_, d)| d)).any(|d| inside(p, d));
    f.codex_home = match aw_codex {
        Some(c) if !ours(&c) => CodexHomePlan { from: Some(fwd(&c)), to: fwd(&c), action: "kept".into() },
        Some(c) if c.is_dir() => {
            let xen_in_use = std::fs::read_dir(&xen_codex).is_ok_and(|mut rd| rd.next().is_some());
            CodexHomePlan { from: Some(fwd(&c)), to: fwd(&xen_codex), action: if xen_in_use { "kept-xenolith" } else { "move" }.into() }
        }
        _ => CodexHomePlan { from: None, to: fwd(&xen_codex), action: "absent".into() },
    };
    let codex_target = f.codex_home.to.clone();
    let (carry, dropped) = carry(&f.config, &xen_cfg, &wiki, &codex_target);
    f.carry = carry;
    f.dropped = dropped;

    // Registrations, processes, the port, the apps.
    let (regs, bad) = registrations(ctx, &f.codex_plugin_id);
    for b in bad {
        f.blockers.push(format!("{b}. Fix it (or restore its backup), then run again; nothing was changed"));
    }
    f.approve = regs.iter().any(|r| r.kind == "Claude Code allow rules" || r.kind == "Codex approval") || !regs.iter().any(|r| r.kind.contains("plugin") || r.kind == "personal marketplace");
    f.registrations = regs;
    f.processes = aw_processes(ctx, &ctx.sys.processes());
    let (busy, health) = ctx.sys.port(ctx.port());
    f.port_busy = busy;
    if busy && health.as_ref().is_none_or(|h| h["name"] != "xenolith") {
        f.blockers.push(format!("port {} is in use by another program. Stop it, or pass --port with a free port.", ctx.port()));
    }
    f.desktop_running = !lay.desktop_configs.is_empty() && ctx.sys.desktop_running();
    f.claude_cli = ctx.sys.which("claude").is_some();
    f.codex_cli = ctx.sys.which("codex").is_some();
    if lay.os == "windows" {
        f.aw_service = win_service(ctx.sys);
        f.aw_task = win_task(ctx.sys);
        f.aw_run_value = win_run_value(ctx.sys);
        if let Some(pkg) = ctx.sys.packaged() {
            f.blockers.push(format!("This terminal runs inside the app package {pkg}, which captures what it writes to AppData. Run it from a normal terminal: Win+R, cmd."));
        }
    }
    f.found = f.found || !f.registrations.is_empty();

    // Files the migration edits must be text it can read back exactly.
    for (file, _) in tracked_files(lay, f.wiki.exists.then_some(wiki.as_path())) {
        if let Err(e) = edits::read(&file) {
            f.blockers.push(e);
        }
    }

    // What else to know.
    if f.wiki.spaces_dir {
        f.blockers.push(format!("{}/spaces exists. Xenolith keeps other spaces in that folder and would not read it as part of your wiki. Rename it, then run again.", fwd(&wiki)));
    }
    if f.found && !f.wiki.exists && !wiki.exists() {
        f.blockers.push(format!("the wiki folder {} does not exist. If it moved, pass --wiki-dir <folder>.", fwd(&wiki)));
    }
    if !lay.aw_vars.is_empty() {
        f.warnings.push(format!("{} set in this shell. Xenolith ignores them; remove them from your shell profile after the move.", lay.aw_vars.join(", ")));
    }
    if f.desktop_running && f.registrations.iter().any(|r| r.kind == "Claude desktop") {
        f.warnings.push("Claude desktop is running: it would write its config back, so its agent-wiki entry waits until you quit it (tray icon > Quit) and run --resume.".into());
    }
    if !f.claude_cli && f.registrations.iter().any(|r| r.kind.starts_with("Claude Code plugin") || r.kind == "Claude Code marketplace") {
        f.warnings.push("the claude CLI is not on PATH, so the Claude Code plugin agent-wiki@agent-wiki-local stays: remove it in Claude Code with /plugin.".into());
    }
    if !f.codex_cli && f.registrations.iter().any(|r| r.kind == "Codex plugin") {
        f.warnings.push(format!("the codex CLI is not on PATH, so the Codex plugin {} stays: remove it from the ChatGPT app's Plugins.", f.codex_plugin_id));
    }
    for l in f.wiki.locks.iter().filter(|l| l.live) {
        f.warnings.push(format!("the wiki's {} lock is held by {}; the migration waits for it after stopping agent-wiki", l.name, l.owner));
    }
    let sessions = f.processes.iter().filter(|p| p.kind == "session").count();
    if sessions > 0 {
        f.warnings.push(format!("{sessions} agent-wiki server(s) started by open Claude Code or Codex sessions still run; they stop when those sessions end"));
    }
    if f.wiki.link.is_some() {
        f.notes.push(format!("the wiki folder is a link to {}; Xenolith uses it through the link", f.wiki.link.as_deref().unwrap_or("")));
    }
    if f.wiki.protocol == "edited" {
        f.notes.push("PROTOCOL.md is your own edit, so it is kept.".into());
    }
    if let Some(at) = &f.uninstalled_at {
        f.notes.push(format!("agent-wiki was uninstalled on {at}; its folders and the wiki remain"));
    }
    if f.pre13 || (lay.legacy.is_dir() && f.folders.len() > 1) {
        f.notes.push(format!("{} (from before 1.3) is there too; --finish removes it", fwd(&lay.legacy)));
    }
    if !lay.captured.is_empty() {
        f.notes.push(format!("{} cop(ies) of agent-wiki's folders were captured by an app package; --finish removes them", lay.captured.len()));
    }
    if !f.approve {
        f.notes.push("agent-wiki asked before each tool call, so Xenolith is installed with --no-approve".into());
    }
    f
}

/// The plan, in plain words.
pub fn plan(ctx: &Ctx, f: &Found) -> Vec<String> {
    let wiki = f.wiki.path.clone().unwrap_or_default();
    let tracked = tracked_files(&ctx.lay, f.wiki.exists.then(|| Path::new(&wiki)));
    let mut stop: Vec<String> = vec![];
    if !f.programs.trays.is_empty() {
        stop.push("the tray".into());
    }
    match ctx.lay.os.as_str() {
        "macos" => stop.extend(f.registrations.iter().filter(|r| r.kind == "LaunchAgent").map(|r| format!("LaunchAgent {}", r.key))),
        "windows" => {
            if f.aw_service.is_some() {
                stop.push(format!("the {AW_SERVICE} service"));
            }
        }
        _ => stop.extend(f.registrations.iter().filter(|r| r.kind == "user unit").map(|r| r.key.clone())),
    }
    if f.era == "node" {
        stop.push("the curator's Codex app-server daemon".into());
    }
    let mut out = vec![
        format!("1. Write a journal and back up {} files with their SHA-256 to {}", tracked.iter().filter(|(p, _)| p.exists()).count(), fwd(&ctx.lay.work)),
        format!("2. Stop agent-wiki ({}), pause the curator, and wait until the wiki's locks are free", if stop.is_empty() { "nothing runs".to_string() } else { stop.join(", ") }),
        format!("3. Write Xenolith's config.json: the wiki stays where it is ({wiki}), with agent-wiki's settings carried over"),
        match f.codex_home.action.as_str() {
            "move" => format!("4. Move the curator's sign-in from {} to {}", f.codex_home.from.as_deref().unwrap_or(""), f.codex_home.to),
            "kept" => format!("4. Keep the curator's sign-in where it is ({})", f.codex_home.to),
            "kept-xenolith" => format!("4. Keep Xenolith's own curator sign-in ({}); agent-wiki's stays until --finish", f.codex_home.to),
            _ => "4. No curator sign-in to move".to_string(),
        },
        format!(
            "5. Run xenolith install --wiki-dir {wiki}{}{}",
            if f.approve { "" } else { " --no-approve" },
            if f.aw_service.is_some() { " (one UAC prompt, which also removes the AgentWiki service)" } else { "" }
        ),
    ];
    out.push(match f.registrations.len() {
        0 => "6. Remove agent-wiki's registrations: none found".to_string(),
        n => format!("6. Remove agent-wiki's {n} registrations listed above, as its own uninstaller would (its programs stay until --finish)"),
    });
    out.push("7. Check: the service and the tools answer on the adopted folder, every page, log, note, held change and vector is still there, the curator is signed in, and nothing of agent-wiki's is registered".into());
    out
}

/// Settings as a short JSON object for the report (the carried settings).
pub fn carried_keys(v: &Value) -> Vec<String> {
    v.as_object().map(|o: &Map<String, Value>| o.keys().filter(|k| *k != "wikiDir").cloned().collect()).unwrap_or_default()
}
