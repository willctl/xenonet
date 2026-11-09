//! Headless resume: a Claude Code, Codex or Grok Build session whose
//! process has gone is started again for one turn, with the message as its
//! prompt (Grok Build reads it from a file the deliverer adds):
//! core/delivery/resume.js.
//!
//! ```text
//! claude -p --resume <id> [--settings <listen file>]
//! codex exec resume <id> -
//! grok -r <id> --output-format json --prompt-file <file>
//! ```
//!
//! The router uses this only for a session you allowed (or any, under
//! wake: network), since a turn spends its plan, and never while the
//! session's own process runs: a Claude Code session one holds gets a
//! copy instead (`fork_command`). The message goes
//! in on stdin, never as an argument. The run itself is the shared
//! runner's (`runs`): it finds the program, logs the turn and caps how
//! many go at once.

use super::apps::{self, Ways};
use xenonet_adapter::apps::App;

/// What a plan asks before a turn starts.
pub trait Host {
    /// Whether a resumed turn for the session is under way.
    fn running(&mut self, key: &str) -> bool;
    /// Whether the folder is there.
    fn exists(&mut self, path: &str) -> bool;
}

/// A turn's command: the app whose CLI the runner starts, and its
/// arguments as they are (the runner quotes them once for the platform).
pub type Command = (String, Vec<String>);

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

/// The apps whose sessions can be resumed, each with its command
/// (`apps`).
fn resumed() -> impl Iterator<Item = (&'static App, Ways)> {
    apps::apps().into_iter().filter(|(_, w)| w.resume.is_some())
}

/// The environment a resumed turn runs with: XenoNet's own, without what
/// marks a process as part of another session (Claude Code's and Codex's
/// session variables, XenoNet's channel switch and actor tokens). Where
/// sessions are kept (CLAUDE_CONFIG_DIR, CODEX_HOME: each app's roots.env)
/// stays. Only the names decide, so the values can be of any type.
pub fn resume_env<V>(env: Vec<(String, V)>) -> Vec<(String, V)> {
    let keep: Vec<&str> = resumed().filter_map(|(a, _)| a.roots.as_ref().map(|r| r.env.as_str())).collect();
    let marks: Vec<&str> = resumed().filter_map(|(_, w)| w.marks).chain(["XENONET_"]).collect();
    env.into_iter().filter(|(k, _)| keep.contains(&k.as_str()) || !marks.iter().any(|m| k.starts_with(m))).collect()
}

/// The command for one resumed turn, or None for an app or id it can't
/// resume. `settings` is the listen settings file a Claude Code turn runs
/// with.
pub fn resume_command(agent: &str, settings: Option<&str>) -> Option<Command> {
    // split(/:(.*)/s): the app, and everything after the first colon.
    let (app, id) = agent.split_once(':')?;
    if !is_uuid(id) {
        return None;
    }
    let resume = resumed().find(|(a, _)| a.id == app)?.1.resume?;
    Some(resume(id, settings))
}

/// The command for a copy of a Claude Code session that a process holds:
/// its whole context under a new id, which the stream's first event
/// names. None for any other app or id.
///
/// ```text
/// claude -p --resume <id> --fork-session --output-format stream-json --verbose [--settings <listen file>]
/// ```
pub fn fork_command(agent: &str, settings: Option<&str>) -> Option<Command> {
    let id = agent.strip_prefix("claude:").filter(|id| is_uuid(id))?;
    let mut args: Vec<String> = ["-p", "--resume", id, "--fork-session", "--output-format", "stream-json", "--verbose"].into_iter().map(str::to_string).collect();
    if let Some(s) = settings.filter(|s| !s.is_empty()) {
        args.push("--settings".into());
        args.push(s.to_string());
    }
    Some(("claude".into(), args))
}

/// A Claude Code turn XenoNet runs asks its permission prompts of you,
/// through the xenonet MCP server (`server`, its name in the session's
/// settings) and its `permission` tool, as a started session does. Other
/// apps' arguments are as they are.
pub fn with_permission_tool(app: &str, mut args: Vec<String>, server: &str) -> Vec<String> {
    if app == "claude" {
        args.push("--permission-prompt-tool".into());
        args.push(format!("mcp__{server}__permission"));
    }
    args
}

fn folder_there(h: &mut dyn Host, cwd: Option<&str>) -> bool {
    cwd.is_some_and(|c| !c.is_empty() && h.exists(c))
}

/// A fork's deliver before anything starts, as `plan`, but while the
/// session runs too: that's what it's for.
pub fn plan_fork(h: &mut dyn Host, agent: &str, settings: Option<&str>, cwd: Option<&str>) -> Result<Command, &'static str> {
    let Some(command) = fork_command(agent, settings) else {
        return Err("only Claude Code sessions can be forked");
    };
    if !folder_there(h, cwd) {
        return Err("its folder isn't there");
    }
    Ok(command)
}

/// What a deliver comes to before anything starts: Err(why) it can't, or
/// the command the runner starts.
pub fn plan(h: &mut dyn Host, agent: &str, settings: Option<&str>, cwd: Option<&str>) -> Result<Command, &'static str> {
    let Some(command) = resume_command(agent, settings) else {
        return Err("only Claude Code, Codex and Grok Build sessions can be resumed");
    };
    if h.running(agent) {
        return Err("a resumed turn is under way");
    }
    if !folder_there(h, cwd) {
        return Err("its folder isn't there");
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_sessions_own_id_resumes() {
        xenonet_platforms::install();
        let id = "0190d3e0-0000-7000-8000-00000000ab01";
        assert_eq!(resume_command(&format!("codex:{id}"), None).unwrap().1.len(), 4);
        assert_eq!(resume_command(&format!("claude:{id}"), Some("/s")).unwrap().1.len(), 5);
        assert!(resume_command("claude:abc", None).is_none());
        assert!(resume_command(&format!("opencode:{id}"), None).is_none());
        let env = resume_env(vec![("PATH".into(), "/bin"), ("CLAUDE_PID".into(), "1"), ("CLAUDE_CONFIG_DIR".into(), "/c")]);
        assert_eq!(env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["PATH", "CLAUDE_CONFIG_DIR"]);
        let env = resume_env(["CLAUDECODE", "CODEX_HOME", "CODEX_THREAD_ID", "CODEXX", "XENONET_PORT", "OPENCODE_X", "HOME"].iter().map(|k| ((*k).to_string(), "x")).collect());
        assert_eq!(env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["CODEX_HOME", "CODEXX", "OPENCODE_X", "HOME"]);
        // Grok Build reads its prompt from a file the deliverer adds.
        let (grok, args) = resume_command(&format!("grok:{id}"), None).unwrap();
        assert_eq!((grok.as_str(), args), ("grok", vec!["-r".into(), id.to_string(), "--output-format".into(), "json".into(), "--prompt-file".to_string()]));
        let env = resume_env(vec![("GROK_SESSION_ID".into(), "x"), ("GROK_HOME".into(), "/g")]);
        assert_eq!(env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["GROK_HOME"]);
    }

    #[test]
    fn a_copy_forks_only_from_a_claude_code_session() {
        xenonet_platforms::install();
        let id = "0190d3e0-0000-7000-8000-00000000ab01";
        let (cmd, args) = fork_command(&format!("claude:{id}"), Some("/s/listen.json")).unwrap();
        assert_eq!(
            (cmd.as_str(), args),
            ("claude", ["-p", "--resume", id, "--fork-session", "--output-format", "stream-json", "--verbose", "--settings", "/s/listen.json"].map(str::to_string).to_vec())
        );
        assert!(fork_command(&format!("codex:{id}"), None).is_none());
        assert!(fork_command("claude:abc", None).is_none());
    }

    #[test]
    fn a_resumed_or_forked_claude_code_turn_asks_its_prompts_of_you() {
        xenonet_platforms::install();
        let (_, fork) = fork_command("claude:0190d3e0-0000-7000-8000-00000000ab01", None).unwrap();
        let asked = with_permission_tool("claude", fork, "xenonet-dev");
        assert_eq!(asked[asked.len() - 2..], ["--permission-prompt-tool".to_string(), "mcp__xenonet-dev__permission".to_string()]);
        let codex = vec!["exec".to_string(), "-".to_string()];
        assert_eq!(with_permission_tool("codex", codex.clone(), "xenonet"), codex, "only Claude Code has a permission prompt tool");
    }

    /// A plan's folder and runner, in memory.
    struct Fake {
        running: bool,
    }

    impl Host for Fake {
        fn running(&mut self, _: &str) -> bool {
            self.running
        }
        fn exists(&mut self, path: &str) -> bool {
            path == "/w/atlas"
        }
    }

    #[test]
    fn a_plan_wants_the_folder_and_no_turn_under_way() {
        xenonet_platforms::install();
        let agent = "claude:0190d3e0-0000-7000-8000-00000000ab01";
        let mut h = Fake { running: false };
        assert!(plan(&mut h, agent, None, Some("/w/atlas")).is_ok());
        assert_eq!(plan(&mut h, agent, None, Some("/w/gone")), Err("its folder isn't there"));
        assert_eq!(plan(&mut h, agent, None, None), Err("its folder isn't there"));
        assert_eq!(plan(&mut h, "opencode:ses-1", None, Some("/w/atlas")), Err("only Claude Code, Codex and Grok Build sessions can be resumed"));
        assert_eq!(plan_fork(&mut h, "codex:0190d3e0-0000-7000-8000-00000000ab01", None, Some("/w/atlas")), Err("only Claude Code sessions can be forked"));
        h.running = true;
        assert_eq!(plan(&mut h, agent, None, Some("/w/atlas")), Err("a resumed turn is under way"));
        assert!(plan_fork(&mut h, agent, None, Some("/w/atlas")).is_ok(), "a fork goes while the session runs");
    }
}
