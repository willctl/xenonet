//! Headless resume: a Claude Code or Codex session whose process has gone
//! is started again for one turn, with the message as its prompt:
//! core/delivery/resume.js.
//!
//! ```text
//! claude -p --resume <id> [--settings <listen file>]
//! codex exec resume <id> -
//! ```
//!
//! The router uses this only for a session you allowed, since a turn
//! spends its plan, and never while the session's own process runs. The
//! message goes in on stdin, never as an argument. Starting the process and
//! watching it are the twin's.

use crate::peer::{Calls, Result};
use crate::sql::Arg;
use crate::store::js_text;

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

/// The environment a resumed turn runs with: XenoNet's own, without what
/// marks a process as part of another session (Claude Code's and Codex's
/// session variables, XenoNet's channel switch and actor tokens). Where
/// sessions are kept (CLAUDE_CONFIG_DIR, CODEX_HOME) stays.
pub fn resume_env(env: Vec<(String, Arg)>) -> Vec<(String, Arg)> {
    env.into_iter().filter(|(k, _)| k == "CLAUDE_CONFIG_DIR" || k == "CODEX_HOME" || !(k.starts_with("CLAUDE") || k.starts_with("CODEX_") || k.starts_with("XENONET_"))).collect()
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
    match app {
        "claude" => {
            let mut args = vec![Arg::text("-p"), Arg::text("--resume"), Arg::text(id)];
            if settings.truthy() {
                args.push(Arg::text("--settings"));
                args.push(settings.clone());
            }
            Some(("claude".into(), args))
        }
        "codex" => Some(("codex".into(), vec![Arg::text("exec"), Arg::text("resume"), Arg::text(id), Arg::text("-")])),
        _ => None,
    }
}

/// What a deliver comes to before anything starts: Err(why) it can't, or
/// the command and its arguments (quoted for Windows, whose claude.cmd
/// runs only through a shell). The twin's "running" and "exists" say
/// whether a resumed turn is under way and whether its folder is there.
pub fn plan(c: &mut dyn Calls, agent: &Arg, settings: &Arg, cwd: &Arg, windows: bool) -> Result<std::result::Result<(String, Vec<Arg>), &'static str>> {
    let Some((cmd, args)) = resume_command(agent, settings) else {
        return Ok(Err("only Claude Code and Codex sessions can be resumed"));
    };
    if c.call("", "running", vec![agent.clone()])?.truthy() {
        return Ok(Err("a resumed turn is under way"));
    }
    if !cwd.truthy() || !c.call("", "exists", vec![cwd.clone()])?.truthy() {
        return Ok(Err("its folder isn't there"));
    }
    let args = if windows {
        args.into_iter()
            .map(|a| {
                let t = js_text(&a);
                if crate::re!(r"\s").is_match(&t) {
                    Arg::text(format!("\"{t}\""))
                } else {
                    a
                }
            })
            .collect()
    } else {
        args
    };
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
    }
}
