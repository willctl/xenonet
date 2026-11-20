//! Claude Code's own sandbox, driven by XenoNet: one settings file per run
//! (passed with --settings, with the user's, the project's and the local
//! settings left out), and one MCP configuration with XenoNet's server
//! alone. The sandbox wraps shell commands only, so the same file carries
//! the permission rules that hold the file and web tools.
//!
//! Keys, as Claude Code 2.1.327 has them: sandbox.enabled,
//! sandbox.failIfUnavailable, sandbox.allowUnsandboxedCommands,
//! sandbox.filesystem.{allowWrite,denyRead,allowRead},
//! sandbox.network.{allowedDomains,strictAllowlist},
//! permissions.{blockReadsOutsideWorkingDirectories,
//! blockWritesOutsideWorkingDirectories,allow,deny}.

use serde_json::{json, Map, Value};
use xenonet_adapter::apps::app;
use xenonet_adapter::delivery::{Plan, SandboxWays};

pub fn ways() -> SandboxWays {
    SandboxWays { unavailable, files: Some(files), words }
}

/// Native Windows has no sandbox in Claude Code: it runs commands
/// unsandboxed there, so a start that asks for one isn't made. Under WSL2
/// it is Linux, which has one.
fn unavailable(_: &Plan<'_>, windows: bool) -> Option<String> {
    windows.then(|| "Claude Code has no sandbox on native Windows (it would run every command unsandboxed): run it under WSL2 for a sandboxed start".to_string())
}

fn words(p: &Plan<'_>) -> String {
    let net = if p.domains.is_empty() { "no network".to_string() } else { format!("the network only at {}", p.domains.join(", ")) };
    format!("It runs in Claude Code's sandbox: its commands write only in {}, read none of the home folder but that folder, and reach {net}.", p.folder)
}

/// The settings file and the MCP configuration of one run.
fn files(p: &Plan<'_>, listen: Option<&Value>, server: &str) -> (Value, Value) {
    let mut allow: Vec<Value> = p.domains.iter().map(|d| json!(format!("WebFetch(domain:{d})"))).collect();
    allow.sort_by_key(Value::to_string);
    // The web tools aren't sandboxed: with no domain given they are denied
    // outright, and with some, a fetch of any other asks, which no delegation
    // answers.
    let mut deny: Vec<Value> = vec![json!("WebSearch")];
    if p.domains.is_empty() {
        deny.push(json!("WebFetch"));
    }
    let mut settings = json!({
        "sandbox": {
            "enabled": true,
            "failIfUnavailable": true,
            "allowUnsandboxedCommands": false,
            "filesystem": { "allowWrite": [p.folder], "denyRead": [p.home], "allowRead": [p.folder] },
            "network": { "allowedDomains": p.domains, "strictAllowlist": true },
        },
        "permissions": { "blockReadsOutsideWorkingDirectories": true, "blockWritesOutsideWorkingDirectories": true, "allow": allow, "deny": deny },
        "hooks": Value::Object(Map::new()),
    });
    // XenoNet's hooks, which the settings left out would have brought, and
    // the one that has the session listen after each turn.
    let session_start = json!({ "type": "command", "command": format!("XENONET_PORT={} {} hook claude", p.port, xenonet_adapter::connect::quote(p.bin)), "timeout": 10 });
    let ours = app("claude").map(|a| crate::connect::hook_entries(a, &format!("http://127.0.0.1:{}/api/hook/claude", p.port), &session_start));
    for source in [ours.as_ref(), listen.and_then(|l| l.get("hooks"))].into_iter().flatten() {
        for (event, groups) in source.as_object().into_iter().flatten() {
            if let (Some(add), Some(h)) = (groups.as_array(), settings["hooks"].as_object_mut()) {
                let entry = h.entry(event.clone()).or_insert_with(|| json!([]));
                if let Some(list) = entry.as_array_mut() {
                    list.extend(add.iter().cloned());
                }
            }
        }
    }
    let mcp = json!({ "mcpServers": { server: { "type": "stdio", "command": p.bin, "args": ["mcp"], "env": { "XENONET_PORT": p.port } } } });
    (settings, mcp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan<'a>(domains: &'a [String]) -> Plan<'a> {
        Plan { folder: "/home/me/atlas", home: "/home/me", domains, port: "47840", bin: "/usr/bin/xenonet" }
    }

    #[test]
    fn the_settings_hold_the_run_to_its_folder_and_its_domains() {
        let domains = vec!["github.com".to_string()];
        let (s, mcp) = files(&plan(&domains), Some(&json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "http", "url": "http://x/listen" }] }] } })), "xenonet");
        assert_eq!((&s["sandbox"]["enabled"], &s["sandbox"]["failIfUnavailable"], &s["sandbox"]["allowUnsandboxedCommands"]), (&json!(true), &json!(true), &json!(false)));
        assert_eq!(s["sandbox"]["filesystem"]["allowWrite"], json!(["/home/me/atlas"]));
        assert_eq!(s["sandbox"]["filesystem"]["denyRead"], json!(["/home/me"]));
        assert_eq!(s["sandbox"]["network"]["allowedDomains"], json!(["github.com"]));
        assert_eq!(s["permissions"]["allow"], json!(["WebFetch(domain:github.com)"]));
        assert_eq!(s["permissions"]["deny"], json!(["WebSearch"]));
        assert!(s["hooks"]["Stop"].as_array().unwrap().len() >= 2, "its own hook for Stop and the listening one");
        assert!(s["hooks"]["PreToolUse"].is_array(), "the hooks that put the run on the map: {s}");
        assert_eq!(mcp["mcpServers"]["xenonet"]["env"], json!({ "XENONET_PORT": "47840" }));
        let (none, _) = files(&plan(&[]), None, "xenonet");
        assert_eq!(none["permissions"]["deny"], json!(["WebSearch", "WebFetch"]));
        assert_eq!(none["sandbox"]["network"]["allowedDomains"], json!([]));
    }

    #[test]
    fn native_windows_has_no_sandbox_to_ask_for() {
        assert!(unavailable(&plan(&[]), true).is_some_and(|w| w.contains("WSL2")));
        assert!(unavailable(&plan(&[]), false).is_none());
    }
}
