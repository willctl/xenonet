//! Claude Code's own sandbox, driven by XenoNet: one settings file per run
//! (passed with --settings, with the user's, the project's and the local
//! settings left out), and one MCP configuration with XenoNet's server
//! alone. The sandbox wraps shell commands only, so the same file carries
//! the permission rules that hold the file and web tools.
//!
//! Keys, as Claude Code 2.1.327 has them: sandbox.enabled,
//! sandbox.failIfUnavailable, sandbox.allowUnsandboxedCommands,
//! sandbox.autoAllowBashIfSandboxed, sandbox.credentials.envVars (deny: the
//! variable is unset for sandboxed commands; CLAUDE_CODE_SUBPROCESS_ENV_SCRUB
//! is not used, since it also strips the environment of hooks and MCP
//! servers),
//! sandbox.filesystem.{allowWrite,denyRead,allowRead},
//! sandbox.network.{allowedDomains,strictAllowlist},
//! permissions.{blockReadsOutsideWorkingDirectories,
//! blockWritesOutsideWorkingDirectories,allow,deny}.

use serde_json::{json, Map, Value};
use xenonet_adapter::apps::app;
use xenonet_adapter::delivery::{Plan, SandboxFiles, SandboxWays};

pub fn ways() -> SandboxWays {
    SandboxWays { unavailable, files: Some(files), words, turn, reads_whole_disk: false }
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

/// A turn of a sandboxed run loads XenoNet's settings and servers and none
/// of the repo's, which on a guest is someone else's content, and whose
/// hooks and servers would run outside the sandbox: the settings file the
/// turn had is replaced by the run's, and the user's, the project's and the
/// local settings and every other MCP server are left out.
fn turn(f: &SandboxFiles<'_>, args: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--settings" {
            it.next();
        } else {
            out.push(a);
        }
    }
    out.extend(["--settings", f.settings, "--setting-sources=", "--strict-mcp-config", "--mcp-config", f.mcp].map(str::to_string));
    out
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
            // A sandboxed command still asks, as any other does: the person,
            // or the delegation's rules, decide each one.
            "autoAllowBashIfSandboxed": false,
            // The sign-in is in the run's environment for Claude Code
            // itself; a command it runs doesn't get it.
            "credentials": { "envVars": [{ "name": crate::SIGN_IN_VARIABLE, "mode": "deny" }] },
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
            // An event's groups: a list, or the one group connect's set has.
            let add: Vec<Value> = groups.as_array().cloned().unwrap_or_else(|| vec![groups.clone()]);
            if let Some(list) = settings["hooks"].as_object_mut().map(|h| h.entry(event.clone()).or_insert_with(|| json!([]))).and_then(Value::as_array_mut) {
                list.extend(add);
            }
        }
    }
    let mcp = json!({ "mcpServers": { server: { "type": "stdio", "command": p.bin, "args": ["mcp"], "env": { "XENONET_PORT": p.port } } } });
    (settings, mcp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(domains: &[String]) -> Plan<'_> {
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
    fn a_command_in_the_sandbox_cannot_read_the_sign_in_and_bash_is_not_allowed_unasked() {
        let (s, _) = files(&plan(&[]), None, "xenonet");
        // The variable the app's runs sign in with, as its adapter names it.
        let app = app("claude").expect("claude is an app");
        let variable = xenonet_adapter::Platform::delivery(&crate::Adapter(app)).and_then(|w| w.sign_in).map(|s| s.variable).expect("claude's runs sign in");
        assert_eq!(variable, "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(s["sandbox"]["credentials"]["envVars"], json!([{ "name": variable, "mode": "deny" }]), "{s}");
        // Without this, a sandboxed Bash runs without a question.
        assert_eq!(s["sandbox"]["autoAllowBashIfSandboxed"], json!(false), "{s}");
    }

    #[test]
    fn a_turn_swaps_the_listening_settings_for_the_runs_and_leaves_the_repos_out() {
        let p = plan(&[]);
        let f = SandboxFiles { plan: p, settings: "/s/a.json", mcp: "/s/m.json", server: "xenonet" };
        let args = turn(&f, ["-p", "--resume", "x", "--settings", "/s/listen.json", "--permission-prompt-tool", "t"].map(str::to_string).to_vec());
        assert_eq!(
            args,
            ["-p", "--resume", "x", "--permission-prompt-tool", "t", "--settings", "/s/a.json", "--setting-sources=", "--strict-mcp-config", "--mcp-config", "/s/m.json"]
        );
    }

    #[test]
    fn native_windows_has_no_sandbox_to_ask_for() {
        assert!(unavailable(&plan(&[]), true).is_some_and(|w| w.contains("WSL2")));
        assert!(unavailable(&plan(&[]), false).is_none());
    }
}
