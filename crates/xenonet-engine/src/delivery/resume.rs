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
use crate::peer::{Calls, Result};
use crate::sql::Arg;
use crate::store::js_text;
use xenonet_base::apps::App;

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

/// The apps whose sessions can be resumed, each with its command
/// (`apps`).
fn resumed() -> impl Iterator<Item = &'static (&'static App, Ways)> {
    apps::apps().iter().filter(|(_, w)| w.resume.is_some())
}

/// The environment a resumed turn runs with: XenoNet's own, without what
/// marks a process as part of another session (Claude Code's and Codex's
/// session variables, XenoNet's channel switch and actor tokens). Where
/// sessions are kept (CLAUDE_CONFIG_DIR, CODEX_HOME: each app's roots.env)
/// stays.
pub fn resume_env(env: Vec<(String, Arg)>) -> Vec<(String, Arg)> {
    let keep: Vec<&str> = resumed().filter_map(|(a, _)| a.roots.as_ref().map(|r| r.env.as_str())).collect();
    let marks: Vec<&str> = resumed().filter_map(|(_, w)| w.marks).chain(["XENONET_"]).collect();
    env.into_iter().filter(|(k, _)| keep.contains(&k.as_str()) || !marks.iter().any(|m| k.starts_with(m))).collect()
}

/// The command for one resumed turn, or None for an app or id it can't
/// resume.
pub fn resume_command(agent: &Arg, settings: &Arg) -> Option<(String, Vec<Arg>)> {
    let s = js_text(agent);
    // split(/:(.*)/s): the app, and everything after the first colon.
    let (app, id) = s.split_once(':')?;
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
pub fn fork_command(agent: &Arg, settings: &Arg) -> Option<(String, Vec<Arg>)> {
    let s = js_text(agent);
    let id = s.strip_prefix("claude:").filter(|id| is_uuid(id))?;
    let mut args: Vec<Arg> = ["-p", "--resume", id, "--fork-session", "--output-format", "stream-json", "--verbose"].into_iter().map(Arg::text).collect();
    if settings.truthy() {
        args.push(Arg::text("--settings"));
        args.push(settings.clone());
    }
    Some(("claude".into(), args))
}

/// A fork's deliver before anything starts, as `plan`, but while the
/// session runs too: that's what it's for.
pub fn plan_fork(c: &mut dyn Calls, agent: &Arg, settings: &Arg, cwd: &Arg) -> Result<std::result::Result<(String, Vec<Arg>), &'static str>> {
    let Some((cmd, args)) = fork_command(agent, settings) else {
        return Ok(Err("only Claude Code sessions can be forked"));
    };
    if !cwd.truthy() || !c.call("", "exists", vec![cwd.clone()])?.truthy() {
        return Ok(Err("its folder isn't there"));
    }
    Ok(Ok((cmd, args)))
}

/// What a deliver comes to before anything starts: Err(why) it can't, or
/// the app whose CLI the runner starts and its arguments, as they are: the
/// runner quotes them once for the platform. The twin's "running" and
/// "exists" say whether a resumed turn is under way and whether its folder
/// is there.
pub fn plan(c: &mut dyn Calls, agent: &Arg, settings: &Arg, cwd: &Arg) -> Result<std::result::Result<(String, Vec<Arg>), &'static str>> {
    let Some((cmd, args)) = resume_command(agent, settings) else {
        return Ok(Err("only Claude Code, Codex and Grok Build sessions can be resumed"));
    };
    if c.call("", "running", vec![agent.clone()])?.truthy() {
        return Ok(Err("a resumed turn is under way"));
    }
    if !cwd.truthy() || !c.call("", "exists", vec![cwd.clone()])?.truthy() {
        return Ok(Err("its folder isn't there"));
    }
    Ok(Ok((cmd, args)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_sessions_own_id_resumes() {
        let id = "0190d3e0-0000-7000-8000-00000000ab01";
        assert_eq!(resume_command(&Arg::text(format!("codex:{id}")), &Arg::Null).unwrap().1.len(), 4);
        assert_eq!(resume_command(&Arg::text(format!("claude:{id}")), &Arg::text("/s")).unwrap().1.len(), 5);
        assert!(resume_command(&Arg::text("claude:abc"), &Arg::Null).is_none());
        assert!(resume_command(&Arg::text(format!("opencode:{id}")), &Arg::Null).is_none());
        let env = resume_env(vec![("PATH".into(), Arg::text("/bin")), ("CLAUDE_PID".into(), Arg::text("1")), ("CLAUDE_CONFIG_DIR".into(), Arg::text("/c"))]);
        assert_eq!(env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["PATH", "CLAUDE_CONFIG_DIR"]);
        let env = resume_env(
            ["CLAUDECODE", "CODEX_HOME", "CODEX_THREAD_ID", "CODEXX", "XENONET_PORT", "OPENCODE_X", "HOME"].iter().map(|k| ((*k).to_string(), Arg::text("x"))).collect(),
        );
        assert_eq!(env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["CODEX_HOME", "CODEXX", "OPENCODE_X", "HOME"]);
        // Grok Build reads its prompt from a file the deliverer adds.
        let (grok, args) = resume_command(&Arg::text(format!("grok:{id}")), &Arg::Null).unwrap();
        assert_eq!((grok.as_str(), args.iter().map(js_text).collect::<Vec<_>>()), ("grok", vec!["-r".into(), id.to_string(), "--output-format".into(), "json".into(), "--prompt-file".to_string()]));
        let env = resume_env(vec![("GROK_SESSION_ID".into(), Arg::text("x")), ("GROK_HOME".into(), Arg::text("/g"))]);
        assert_eq!(env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["GROK_HOME"]);
    }

    #[test]
    fn a_copy_forks_only_from_a_claude_code_session() {
        let id = "0190d3e0-0000-7000-8000-00000000ab01";
        let (cmd, args) = fork_command(&Arg::text(format!("claude:{id}")), &Arg::text("/s/listen.json")).unwrap();
        assert_eq!(
            (cmd.as_str(), args.iter().map(js_text).collect::<Vec<_>>()),
            ("claude", ["-p", "--resume", id, "--fork-session", "--output-format", "stream-json", "--verbose", "--settings", "/s/listen.json"].map(str::to_string).to_vec())
        );
        assert!(fork_command(&Arg::text(format!("codex:{id}")), &Arg::Null).is_none());
        assert!(fork_command(&Arg::text("claude:abc"), &Arg::Null).is_none());
    }
}
