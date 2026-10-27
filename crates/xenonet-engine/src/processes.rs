//! How XenoNet tells each app's processes apart (core/processes.js): the
//! apps the registry gives `processes`, each with its own rules here, by
//! its id. The team that signs an app is the registry's. `procs` reads the
//! processes with these, and `verify` checks that a request in a session's
//! name comes from its app's process.

use crate::procs::slashes;
use crate::re;
use std::sync::OnceLock;
use xenonet_base::apps::App;
use xenonet_base::nodepath::Style;

/// An app's rules.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    /// Whether a program is one of the app's own, from its path (with /
    /// for \) and its file's name.
    pub program: fn(&str, &str) -> bool,
    /// Whether Node or Bun running these arguments runs the app's package.
    pub script: Option<fn(&str) -> bool>,
    /// The app sends its hooks from its own process, which its maker signs,
    /// so they pin a session to that process.
    pub pins_by_hook: bool,
    /// Any of the app's processes may have any of its sessions open, so
    /// they aren't pinned to one.
    pub any_process: bool,
}

// Claude Code: the CLI, the native install under
// ~/.local/share/claude/versions, or the one Claude desktop runs from its
// claude.app; on Windows claude.exe, but for Claude desktop's own, which is
// called that too and lives in its AnthropicClaude folder.
fn claude_program(p: &str, base: &str) -> bool {
    base == "claude" || (base == "claude.exe" && !p.contains("/AnthropicClaude/")) || re!(r"\/claude\/versions\/[^/]+$").is_match(p)
}
fn claude_script(args: &str) -> bool {
    args.contains("@anthropic-ai/claude-code/cli.js")
}

// Codex: the CLI (codex.exe on Windows), or the one the ChatGPT app
// bundles.
fn codex_program(_: &str, base: &str) -> bool {
    base == "codex" || base == "codex.exe" || re!(r"^codex-(aarch64|x86_64)").is_match(base)
}
fn codex_script(args: &str) -> bool {
    args.contains("@openai/codex/")
}

// Grok Build: the grok binary its installer puts in ~/.grok/downloads and
// links from ~/.grok/bin (as grok, and as agent, which Cursor's CLI is
// called too, so that name counts only there).
fn grok_program(p: &str, base: &str) -> bool {
    base == "grok"
        || base == "grok.exe"
        || re!(r"^grok-(macos|linux|windows)-(aarch64|arm64|x86_64|x64)(\.exe)?$").is_match(base)
        || re!(r"\/\.grok\/(bin|downloads)\/[^/]+$").is_match(p)
}

// OpenCode: its one binary, however it was installed. Every OpenCode reads
// the same database and can open any session.
fn opencode_program(_: &str, base: &str) -> bool {
    base == "opencode" || base == "opencode.exe"
}

/// The rules of an app XenoNet has them for, by its id.
pub fn rules(id: &str) -> Option<Rules> {
    let none = Rules { program: |_, _| false, script: None, pins_by_hook: false, any_process: false };
    match id {
        "claude" => Some(Rules { program: claude_program, script: Some(claude_script), pins_by_hook: true, ..none }),
        "codex" => Some(Rules { program: codex_program, script: Some(codex_script), ..none }),
        "grok" => Some(Rules { program: grok_program, ..none }),
        "opencode" => Some(Rules { program: opencode_program, any_process: true, ..none }),
        _ => None,
    }
}

/// An app whose processes XenoNet tells apart.
#[derive(Clone, Debug)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub rules: Rules,
}

/// The apps whose processes XenoNet tells apart, in the registry's order:
/// those with `processes` that have rules. A test can give its own
/// registry and rules.
pub fn agent_apps(registry: &[App], rules: &dyn Fn(&str) -> Option<Rules>) -> Vec<Agent> {
    registry.iter().filter(|a| a.has("processes")).filter_map(|a| Some(Agent { id: a.id.clone(), name: a.name.clone(), rules: rules(&a.id)? })).collect()
}

/// The registry's, read once.
pub fn agents() -> &'static [Agent] {
    static A: OnceLock<Vec<Agent>> = OnceLock::new();
    A.get_or_init(|| agent_apps(xenonet_base::apps::apps(), &rules))
}

/// An app whose processes XenoNet tells apart, by its id.
pub fn agent(id: &str) -> Option<&'static Agent> {
    agents().iter().find(|a| a.id == id)
}

/// The agent app a program is, from its path: the first app whose program
/// it is, in the registry's order. Programs run by Node or Bun are told by
/// their arguments, when given.
pub fn agent_of(program: &str, args: &str) -> Option<&'static str> {
    agent_among(program, args, agents())
}

/// The same, among these apps.
pub fn agent_among<'a>(program: &str, args: &str, agents: &'a [Agent]) -> Option<&'a str> {
    let p = slashes(program);
    let base = &Style::Posix.basename(&p, None);
    if let Some(a) = agents.iter().find(|a| (a.rules.program)(&p, base)) {
        return Some(&a.id);
    }
    if crate::procs::is_node(base) {
        // A package's path, with / for \ as on Windows.
        let args = slashes(args);
        return agents.iter().find(|a| a.rules.script.is_some_and(|s| s(&args))).map(|a| a.id.as_str());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apps_whose_processes_xenonet_tells_apart_are_the_registrys() {
        let ids: Vec<&str> = agents().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["claude", "codex", "grok", "opencode"], "and so the apps verify checks");
        assert_eq!(agents().iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Claude Code", "Codex", "Grok Build", "OpenCode"]);
        for (program, agent) in [
            ("/Users/me/.grok/downloads/grok-macos-aarch64", Some("grok")),
            ("/Users/me/.grok/bin/agent", Some("grok")),
            ("/usr/local/bin/grok", Some("grok")),
            ("C:\\Users\\me\\.grok\\bin\\grok.exe", Some("grok")),
            ("/usr/local/bin/agent", None),
            ("/usr/local/bin/grokker", None),
        ] {
            assert_eq!(agent_of(program, ""), agent, "{program}");
        }
        assert!(agent("claude").unwrap().rules.pins_by_hook && !agent("codex").unwrap().rules.pins_by_hook);
        assert!(agent("opencode").unwrap().rules.any_process && !agent("claude").unwrap().rules.any_process);
        assert!(agent("gemini-cli").is_none() && rules("gemini-cli").is_none());
    }

    #[test]
    fn a_made_up_app_in_a_registry_of_its_own_is_told_by_its_own_rules() {
        let app = |id: &str, has: &[&str]| App { id: id.into(), name: id.to_uppercase(), has: has.iter().map(|s| (*s).to_string()).collect(), ..App::default() };
        let registry = [app("wren", &["processes"]), app("claude", &["processes"]), app("finch", &[]), app("lark", &["processes"])];
        let made_up = |id: &str| match id {
            "wren" => Some(Rules { program: |_, base| base == "wren", script: Some(|args| args.contains("@wren/cli/")), pins_by_hook: false, any_process: false }),
            other => rules(other),
        };
        let agents = agent_apps(&registry, &made_up);
        assert_eq!(agents.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["wren", "claude"], "finch has no processes, lark no rules");
        assert_eq!(agent_among("/usr/local/bin/wren", "", &agents), Some("wren"));
        assert_eq!(agent_among("/usr/local/bin/node", "node /x/@wren/cli/main.js", &agents), Some("wren"));
        assert_eq!(agent_among("/usr/local/bin/claude", "", &agents), Some("claude"));
        assert_eq!(agent_among("/usr/local/bin/codex", "", &agents), None, "not in this registry");
        assert_eq!(agent_among("/usr/local/bin/wrens", "", &agents), None);
    }
}
