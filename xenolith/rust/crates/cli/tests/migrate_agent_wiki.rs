//! `xenolith migrate agent-wiki` end to end, as a person runs it, in a fake home (--sandbox): the
//! real program, the real `xenolith install --sandbox` with a payload folder, and the installed
//! server answering over stdio. Service managers and the apps' CLIs are only recorded, never run.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_xenolith");
const AW_PROTOCOL: &str = include_str!("../src/migrate/testdata/agent-wiki-1.8.0-PROTOCOL.md");
const NOTE: &str = "PRIVATE-NOTE-TEXT: the dentist moved to Thursday";

struct Home {
    root: PathBuf,
    sb: PathBuf,
    payload: PathBuf,
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn pretty(v: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(v).unwrap())
}

fn exe(n: &str) -> String {
    if cfg!(windows) { format!("{n}.exe") } else { n.to_string() }
}

impl Home {
    /// agent-wiki 1.8.0 as its installer left it in a fake home, with the sandbox's folders
    /// (--sandbox points XDG_* and AppData into the fake home).
    fn new(name: &str) -> Home {
        let root = std::env::temp_dir().join(format!("xenolith-migrate-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sb = root.join("home with space");
        // The payload a release ships: this program, the tray (not run in a sandbox), the window, the icons.
        let payload = root.join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::copy(BIN, payload.join(exe("xenolith"))).unwrap();
        write(&payload.join(exe("xenolith-tray")), "tray");
        write(&payload.join("ui").join("index.html"), "<!doctype html>");
        write(&payload.join("icons").join("xenolith-healthy.ico"), "ico");
        let h = Home { root, sb, payload };
        h.agent_wiki();
        h
    }

    fn aw(&self, kind: &str) -> PathBuf {
        let s = &self.sb;
        if cfg!(windows) {
            match kind {
                "config" => s.join("AppData/Roaming/AgentWiki"),
                "data" => s.join("AppData/Local/AgentWiki"),
                "state" => s.join("AppData/Local/AgentWiki/state"),
                _ => s.join("AppData/Local/AgentWiki/logs"),
            }
        } else {
            match kind {
                "config" => s.join(".config/agent-wiki"),
                "data" => s.join(".local/share/agent-wiki"),
                "state" => s.join(".local/state/agent-wiki"),
                _ => s.join(".local/state/agent-wiki/logs"),
            }
        }
    }

    fn xen(&self, kind: &str) -> PathBuf {
        let s = &self.sb;
        if cfg!(windows) {
            match kind {
                "config" => s.join("AppData/Roaming/Xenolith"),
                "data" => s.join("AppData/Local/Xenolith"),
                _ => s.join("AppData/Local/Xenolith/state"),
            }
        } else {
            match kind {
                "config" => s.join(".config/xenolith"),
                "data" => s.join(".local/share/xenolith"),
                _ => s.join(".local/state/xenolith"),
            }
        }
    }

    fn wiki(&self) -> PathBuf {
        self.sb.join("Agent Wiki")
    }

    fn desktop(&self) -> PathBuf {
        let d = if cfg!(windows) {
            self.sb.join("AppData/Roaming/Claude")
        } else if cfg!(target_os = "macos") {
            self.sb.join("Library/Application Support/Claude")
        } else {
            self.sb.join(".config/Claude")
        };
        d.join("claude_desktop_config.json")
    }

    fn agent_wiki(&self) {
        let data = self.aw("data");
        let codex_home = data.join("curator").join("codex-home");
        write(
            &self.aw("config").join("config.json"),
            &pretty(&json!({
                "wikiDir": self.wiki(), "version": "1.8.0", "httpPort": 47821, "writeMode": "curated",
                "curator": { "model": "gpt-6.1-sol", "reasoningEffort": "high", "codexPath": "codex", "codexHome": codex_home },
                "logs": { "retentionDays": 30 }, "search": { "embeddings": { "enabled": true } },
            })),
        );
        write(&self.aw("state").join("install-state.json"), &pretty(&json!({ "version": "1.8.0", "codexPluginId": "agent-wiki@personal", "created": [] })));
        write(&data.join("runtime").join(exe("agent-wiki")), "agent-wiki");
        write(&data.join("tray").join(exe("agent-wiki-tray")), "tray");
        write(&data.join("marketplace").join(".claude-plugin").join("marketplace.json"), "{}");
        write(&codex_home.join("auth.json"), "{}");
        write(&self.aw("logs").join("requests-2026-10-01.jsonl"), "{\"args\":\"clipped\"}\n");
        let w = self.wiki();
        write(&w.join("PROTOCOL.md"), AW_PROTOCOL);
        write(&w.join("index.md"), "# Index\n");
        write(&w.join("pages").join("dentist.md"), &format!("---\ntitle: Dentist\n---\n{NOTE}\n"));
        write(&w.join("log").join("2026").join("2026-10-01.md"), &format!("# 2026-10-01\n{NOTE}\n"));
        write(&w.join("inbox").join("2026-10-20_10-00-00-000-aaaaaa.md"), &format!("---\nid: 2026-10-20_10-00-00-000-aaaaaa\n---\n{NOTE}\n"));
        let block = "<!-- AGENT-WIKI:START (managed by the agent-wiki installer; replaced on re-install) -->\npointer\n<!-- AGENT-WIKI:END -->";
        write(&self.sb.join(".claude").join("CLAUDE.md"), &format!("# Mine\n\n{block}\n"));
        write(&self.sb.join(".claude").join("settings.json"), &pretty(&json!({ "permissions": { "allow": ["Bash(ls)", "mcp__agent-wiki", "mcp__plugin_agent-wiki_agent-wiki"] } })));
        write(&self.sb.join(".codex").join("AGENTS.md"), &format!("{block}\n"));
        write(
            &self.sb.join(".codex").join("config.toml"),
            "model = \"x\"\n\n# agent-wiki: run the wiki tools without confirmation (managed by the agent-wiki installer)\n[plugins.\"agent-wiki@personal\".mcp_servers.agent-wiki]\ndefault_tools_approval_mode = \"approve\"\n",
        );
        write(&self.sb.join(".agents").join("plugins").join("marketplace.json"), &pretty(&json!({ "name": "personal", "plugins": [{ "name": "agent-wiki" }] })));
        write(&self.desktop(), "{\r\n  \"mcpServers\": {\r\n    \"agent-wiki\": {\r\n      \"command\": \"x\"\r\n    }\r\n  }\r\n}\r\n");
        if cfg!(target_os = "macos") {
            for l in ["com.agentwiki.server", "com.agentwiki.curator", "com.agentwiki.tray"] {
                write(&self.sb.join("Library/LaunchAgents").join(format!("{l}.plist")), "<plist/>\n");
            }
        } else if cfg!(unix) {
            for u in ["agent-wiki.service", "agent-wiki-curator.service"] {
                write(&self.sb.join(".config/systemd/user").join(u), "[Service]\n");
            }
        }
    }

    fn migrate(&self, args: &[&str]) -> (i32, String) {
        let mut c = Command::new(BIN);
        c.args(["migrate", "agent-wiki"]).args(args).arg("--sandbox").arg(&self.sb).arg("--from").arg(&self.payload);
        for (k, _) in std::env::vars() {
            if k.starts_with("XENOLITH_") || k.starts_with("AGENT_WIKI_") {
                c.env_remove(k);
            }
        }
        let o = c.output().unwrap();
        (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    }

    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        fn go(base: &Path, d: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                if e.file_type().unwrap().is_dir() {
                    out.insert(format!("{rel}/"), vec![]);
                    go(base, &p, out);
                } else {
                    out.insert(rel, std::fs::read(&p).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        go(&self.sb, &self.sb, &mut out);
        out
    }
}

#[test]
fn dry_run_run_again_and_rollback_in_a_sandbox() {
    let h = Home::new("main");
    let before = h.snapshot();

    let (code, text) = h.migrate(&[]);
    assert_eq!(code, 0, "{text}");
    for want in [
        "dry run (nothing is changed)",
        "Found agent-wiki 1.8.0 (the Rust programs)",
        "1 page, 1 log day, 1 note waiting",
        "Registrations to replace:",
        "Claude desktop: mcpServers.agent-wiki",
        "Ready.",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    let (code, json_text) = h.migrate(&["--json"]);
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&json_text).unwrap();
    assert_eq!(v["ready"], true);
    assert_eq!(v["plan"].as_array().unwrap().len(), 7);
    assert_eq!(h.snapshot(), before, "a dry run writes nothing");

    let (code, text) = h.migrate(&["--run"]);
    assert_eq!(code, 0, "{text}");
    for want in ["[7] Check", "tools: context_start and wiki_start answer", "wiki: unchanged", "Done. Your wiki stays at", "What is left for you:", "context_start", "--rollback", "--finish"] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    let cfg: Value = serde_json::from_str(&read(&h.xen("config").join("config.json"))).unwrap();
    assert_eq!(cfg["wikiDir"].as_str().unwrap().replace('\\', "/"), h.wiki().to_string_lossy().replace('\\', "/"));
    assert_eq!(cfg["search"]["embeddings"]["credential"], "AgentWiki/OpenRouter");
    assert!(PathBuf::from(cfg["curator"]["codexHome"].as_str().unwrap()).join("auth.json").exists());
    let md = read(&h.sb.join(".claude").join("CLAUDE.md"));
    assert!(md.contains("XENOLITH:START") && !md.contains("AGENT-WIKI"), "{md}");
    let desktop = read(&h.desktop());
    assert!(desktop.contains("\"xenolith\"") && !desktop.contains("agent-wiki") && desktop.contains("\r\n"), "{desktop}");
    assert!(read(&h.wiki().join("pages").join("dentist.md")).contains(NOTE), "the wiki is adopted in place");
    let sys_log = read(&h.sb.join(".migrate-sys.log"));
    if cfg!(target_os = "macos") {
        assert!(sys_log.contains("launchctl bootout") && !h.sb.join("Library/LaunchAgents/com.agentwiki.server.plist").exists(), "{sys_log}");
    }
    let work = h.xen("state").join("migrate-agent-wiki");
    for f in ["journal.json", "migrate.log"] {
        assert!(!read(&work.join(f)).contains("PRIVATE-NOTE-TEXT"), "{f} holds no note text");
    }

    let (code, text) = h.migrate(&["--run"]);
    assert!(code == 0 && text.contains("already moved"), "{text}");

    let (code, text) = h.migrate(&["--rollback"]);
    assert_eq!(code, 0, "{text}");
    for (path, was) in &before {
        if path.ends_with('/') || path.starts_with("Agent Wiki/") {
            continue;
        }
        assert_eq!(h.snapshot().get(path), Some(was), "{path} is back as it was\n{text}");
    }
    assert_eq!(read(&h.wiki().join("PROTOCOL.md")), AW_PROTOCOL);
    assert!(!h.xen("config").join("config.json").exists());
}

#[test]
fn finish_waits_for_agent_wiki_then_removes_it_and_moves_the_wiki() {
    let h = Home::new("finish");
    let (code, text) = h.migrate(&["--run"]);
    assert_eq!(code, 0, "{text}");
    // A session still runs agent-wiki's server.
    let agent = h.aw("data").join("runtime").join(exe("agent-wiki"));
    write(&h.sb.join(".fake").join("processes"), &format!("4242 {} serve\n", agent.display()));
    let (code, text) = h.migrate(&["--finish"]);
    assert!(code == 1 && text.contains("pid 4242 (session)") && text.contains("Nothing was deleted"), "{text}");
    std::fs::remove_file(h.sb.join(".fake").join("processes")).unwrap();
    let (code, text) = h.migrate(&["--finish", "--move-wiki", "--json"]);
    assert_eq!(code, 0, "{text}");
    for k in ["config", "data", "state"] {
        assert!(!h.aw(k).exists(), "{k}: {text}");
    }
    let moved = h.sb.join("Xenolith");
    assert!(read(&moved.join("pages").join("dentist.md")).contains(NOTE) && !h.wiki().exists());
    let cfg: Value = serde_json::from_str(&read(&h.xen("config").join("config.json"))).unwrap();
    assert!(cfg["wikiDir"].as_str().unwrap().ends_with("/Xenolith"), "{cfg}");
    let (code, text) = h.migrate(&["--rollback"]);
    assert!(code == 1 && text.contains("--finish already removed"), "{text}");
}

#[test]
fn update_protocol_renames_the_old_tools_in_an_edited_protocol() {
    let h = Home::new("protocol");
    write(&h.wiki().join("PROTOCOL.md"), "# Mine\n\nCall wiki_start, then wiki_search before wiki_log.\n");
    let (code, text) = h.migrate(&["--run"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("--update-protocol"), "{text}");
    let (code, text) = h.migrate(&["--update-protocol"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(read(&h.wiki().join("PROTOCOL.md")), "# Mine\n\nCall context_start, then context_search before context_log.\n");
}

/// The home agent-wiki's own installer leaves (`agent-wiki install --sandbox`), migrated, rolled
/// back to exactly what it was, migrated again and finished. It needs an agent-wiki 1.8.0 build:
/// AGENT_WIKI_BIN=<its rust/target/release/agent-wiki> cargo test --test migrate_agent_wiki.
#[test]
fn what_agent_wikis_own_installer_left() {
    let Some(aw) = std::env::var_os("AGENT_WIKI_BIN").map(PathBuf::from) else {
        eprintln!("skipped: set AGENT_WIKI_BIN to an agent-wiki 1.8.0 build");
        return;
    };
    let h = Home::new("real");
    std::fs::remove_dir_all(&h.sb).unwrap();
    let aw_payload = h.root.join("aw-payload");
    std::fs::create_dir_all(&aw_payload).unwrap();
    std::fs::copy(&aw, aw_payload.join(exe("agent-wiki"))).unwrap();
    write(&aw_payload.join(exe("agent-wiki-tray")), "tray");
    write(&aw_payload.join("ui").join("index.html"), "<!doctype html>");
    write(&aw_payload.join("icons").join("agent-wiki-healthy.ico"), "ico");
    write(&h.sb.join(".claude").join("CLAUDE.md"), "# Mine\n\nBe terse.\n");
    let o = Command::new(aw_payload.join(exe("agent-wiki"))).arg("install").arg("--sandbox").arg(&h.sb).arg("--from").arg(&aw_payload).env_remove("AGENT_WIKI_HOME").output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let wiki = h.sb.join("AgentWiki");
    write(&wiki.join("pages").join("dentist.md"), &format!("---\ntitle: Dentist\n---\n{NOTE}\n"));
    // Everything but the wiki and Xenolith's own folders comes back byte for byte.
    let outside = |h: &Home| -> BTreeMap<String, Vec<u8>> {
        h.snapshot().into_iter().filter(|(k, _)| !k.starts_with("AgentWiki/") && !k.contains("xenolith") && !k.starts_with(".migrate-sys") && !k.starts_with("Xenolith")).collect()
    };
    let before = outside(&h);
    let (code, text) = h.migrate(&["--run"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("agent-wiki: nothing registered any more") && text.contains("tools: context_start and wiki_start answer"), "{text}");
    let (code, text) = h.migrate(&["--rollback"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(outside(&h), before);
    let (code, text) = h.migrate(&["--run"]);
    assert_eq!(code, 0, "{text}");
    let (code, text) = h.migrate(&["--finish", "--move-wiki"]);
    assert_eq!(code, 0, "{text}");
    assert!(read(&h.sb.join("Xenolith").join("pages").join("dentist.md")).contains(NOTE));
}

#[test]
fn usage_errors_change_nothing() {
    let h = Home::new("usage");
    let before = h.snapshot();
    for args in [vec!["--run", "--rollback"], vec!["--bogus"], vec!["--move-wiki"]] {
        let (code, text) = h.migrate(&args);
        assert_eq!(code, 2, "{args:?}: {text}");
        assert!(text.contains("usage: xenolith migrate agent-wiki"));
    }
    let o = Command::new(BIN).args(["migrate"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(h.snapshot(), before);
}
