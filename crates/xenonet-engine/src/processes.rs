//! How XenoNet tells each app's processes apart: the apps the registry
//! gives `processes`, each with the rules its adapter brings
//! (`xenonet_adapter::processes::Rules`). The team that signs an app is
//! the registry's. `procs` reads the processes with these, and `verify`
//! checks that a request in a session's name comes from its app's process.

use crate::procs::slashes;
use xenonet_adapter::apps::App;
use xenonet_base::nodepath::Style;

pub use xenonet_adapter::processes::Rules;

/// An app whose processes XenoNet tells apart.
#[derive(Clone, Debug)]
pub struct Agent<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub rules: Rules,
}

/// The apps whose processes XenoNet tells apart, in the registry's order:
/// those with `processes` that have rules. A test can give its own
/// registry and rules.
pub fn agent_apps<'a>(registry: &'a [App], rules: &dyn Fn(&str) -> Option<Rules>) -> Vec<Agent<'a>> {
    registry.iter().filter(|a| a.has("processes")).filter_map(|a| Some(Agent { id: &a.id, name: &a.name, rules: rules(&a.id)? })).collect()
}

/// The adapters' rules, by app id.
fn installed(id: &str) -> Option<Rules> {
    xenonet_adapter::platforms().get(id).and_then(|p| p.processes())
}

/// The registry's apps with `processes` whose adapter is in place, with
/// its rules.
pub fn agents() -> Vec<Agent<'static>> {
    agent_apps(xenonet_adapter::apps::apps(), &installed)
}

/// An app whose processes XenoNet tells apart, by its id.
pub fn agent(id: &str) -> Option<Agent<'static>> {
    xenonet_adapter::apps::app(id).filter(|a| a.has("processes")).and_then(|a| Some(Agent { id: &a.id, name: &a.name, rules: installed(id)? }))
}

/// The agent app a program is, from its path: the first app whose program
/// it is, in the registry's order. Programs run by Node or Bun are told by
/// their arguments, when given.
pub fn agent_of(program: &str, args: &str) -> Option<&'static str> {
    agent_among(program, args, &agents())
}

/// The same, among these apps.
pub fn agent_among<'a>(program: &str, args: &str, agents: &[Agent<'a>]) -> Option<&'a str> {
    let p = slashes(program);
    let base = &Style::Posix.basename(&p, None);
    if let Some(a) = agents.iter().find(|a| (a.rules.program)(&p, base)) {
        return Some(a.id);
    }
    if crate::procs::is_node(base) {
        // A package's path, with / for \ as on Windows.
        let args = slashes(args);
        return agents.iter().find(|a| a.rules.script.is_some_and(|s| s(&args))).map(|a| a.id);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apps_whose_processes_xenonet_tells_apart_are_the_registrys() {
        xenonet_platforms::install();
        let ids: Vec<&str> = agents().iter().map(|a| a.id).collect();
        assert_eq!(ids, ["claude", "codex", "grok", "opencode"], "and so the apps verify checks");
        assert_eq!(agents().iter().map(|a| a.name).collect::<Vec<_>>(), ["Claude Code", "Codex", "Grok Build", "OpenCode"]);
        for (program, agent) in [
            ("/Users/me/.grok/downloads/grok-macos-aarch64", Some("grok")),
            ("/Users/me/.grok/bin/agent", Some("grok")),
            ("/usr/local/bin/grok", Some("grok")),
            ("C:\\Users\\me\\.grok\\bin\\grok.exe", Some("grok")),
            ("/usr/local/bin/agent", None),
            ("/usr/local/bin/grokker", None),
            // Windows: Claude desktop's own main executable isn't Claude
            // Code, in any case of its folders; the Claude Code it runs is.
            ("C:\\Users\\me\\AppData\\Local\\AnthropicClaude\\app-1.0.0\\claude.exe", None),
            ("C:/users/me/appdata/local/anthropicclaude/APP-1.2.3/claude.exe", None),
            ("C:\\Users\\me\\AppData\\Local\\AnthropicClaude\\app-1.0.0\\resources\\claude-code\\claude.exe", Some("claude")),
            ("C:\\Users\\me\\AppData\\Roaming\\Claude\\claude-code\\2.1.0\\claude.exe", Some("claude")),
            ("C:\\Users\\me\\.local\\bin\\claude.exe", Some("claude")),
            ("C:\\Users\\me\\AppData\\Roaming\\npm\\codex.exe", Some("codex")),
        ] {
            assert_eq!(agent_of(program, ""), agent, "{program}");
        }
        assert!(agent("claude").unwrap().rules.pins_by_hook && !agent("codex").unwrap().rules.pins_by_hook);
        assert!(agent("opencode").unwrap().rules.any_process && !agent("claude").unwrap().rules.any_process);
        assert!(agent("gemini-cli").is_none() && installed("gemini-cli").is_none());
    }

    #[test]
    fn a_made_up_app_in_a_registry_of_its_own_is_told_by_its_own_rules() {
        xenonet_platforms::install();
        let app = |id: &str, has: &[&str]| App { id: id.into(), name: id.to_uppercase(), has: has.iter().map(|s| (*s).to_string()).collect(), ..App::default() };
        let registry = [app("wren", &["processes"]), app("claude", &["processes"]), app("finch", &[]), app("lark", &["processes"])];
        let made_up = |id: &str| match id {
            "wren" => Some(Rules { program: |_, base| base == "wren", script: Some(|args| args.contains("@wren/cli/")), pins_by_hook: false, any_process: false }),
            other => installed(other),
        };
        let agents = agent_apps(&registry, &made_up);
        assert_eq!(agents.iter().map(|a| a.id).collect::<Vec<_>>(), ["wren", "claude"], "finch has no processes, lark no rules");
        assert_eq!(agent_among("/usr/local/bin/wren", "", &agents), Some("wren"));
        assert_eq!(agent_among("/usr/local/bin/node", "node /x/@wren/cli/main.js", &agents), Some("wren"));
        assert_eq!(agent_among("/usr/local/bin/claude", "", &agents), Some("claude"));
        assert_eq!(agent_among("/usr/local/bin/codex", "", &agents), None, "not in this registry");
        assert_eq!(agent_among("/usr/local/bin/wrens", "", &agents), None);
    }
}
