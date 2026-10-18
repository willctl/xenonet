//! Typing a message into the tmux pane a session runs in: the last resort
//! for a harness with no way in, and only with your consent for that
//! session (policy): core/delivery/tmux.js. Before every keystroke XenoNet
//! checks again that the pane's terminal still runs the session's own
//! process in the foreground, so it never types into the wrong place; the
//! text is one line, with no control characters or escape sequences, sent
//! literally (send-keys -l), then Enter.
//!
//! Commands are run by the twin's "exec" ({ ok, out }), as the JS ran them.

use super::policy::sanitize;
use crate::peer::{Calls, Result, Thrown};
use crate::sql::Arg;
use crate::store::js_text;

pub const MAX_TYPED: f64 = 2000.0;

fn exec(c: &mut dyn Calls, cmd: &str, args: &[&str]) -> Result<(bool, String)> {
    let r = c.call("", "exec", vec![Arg::text(cmd), Arg::List(args.iter().map(|a| Arg::text(*a)).collect())])?;
    Ok((r.field("ok").truthy(), js_text(&r.field("out"))))
}

/// The JS's split(/\s+/).
fn words(s: &str) -> Vec<&str> {
    crate::re!(r"\s+").split(s).collect()
}

/// A pane: its id, process, terminal and command.
#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    pub pane: Arg,
    pub pid: f64,
    pub tty: Arg,
    pub command: Arg,
}

impl Pane {
    pub fn to_arg(&self, session_pid: Option<f64>) -> Arg {
        let mut f = vec![("pane".to_string(), self.pane.clone()), ("pid".into(), Arg::num(self.pid)), ("tty".into(), self.tty.clone()), ("command".into(), self.command.clone())];
        if let Some(p) = session_pid {
            f.push(("sessionPid".into(), Arg::num(p)));
        }
        Arg::Obj(f)
    }
}

/// tmux list-panes' lines, as the JS split them.
pub fn parse_panes(out: &str) -> Vec<Pane> {
    xenonet_base::text::js_trim(out)
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut parts = l.split('\t');
            let mut next = || parts.next().map_or(Arg::Undef, Arg::text);
            let pane = next();
            let pid = next();
            let tty = next();
            let command = next();
            let pid = match &pid {
                Arg::Undef => f64::NAN,
                p => xenonet_base::text::js_number(Some(&p.to_json())),
            };
            Pane { pane, pid, tty, command }
        })
        .collect()
}

pub struct Tmux {
    pub off: bool,
}

impl Tmux {
    pub fn panes(&self, c: &mut dyn Calls) -> Result<Vec<Pane>> {
        let (ok, out) = exec(c, "tmux", &["list-panes", "-a", "-F", "#{pane_id}\t#{pane_pid}\t#{pane_tty}\t#{pane_current_command}"])?;
        if !ok {
            return Ok(Vec::new());
        }
        Ok(parse_panes(&out))
    }

    /// A process's terminal and whether it's in the terminal's foreground
    /// process group (a + in its state).
    fn proc(c: &mut dyn Calls, pid: f64) -> Result<Option<(String, bool)>> {
        let (ok, out) = exec(c, "ps", &["-o", "tty=,stat=", "-p", &js_text(&Arg::num(pid))])?;
        let out = xenonet_base::text::js_trim(&out).to_string();
        if !ok || out.is_empty() {
            return Ok(None);
        }
        let w = words(&out);
        let tty = w[0];
        let Some(stat) = w.get(1) else {
            return Err(Thrown::new("Cannot read properties of undefined (reading 'includes')"));
        };
        let tty = if tty.starts_with("/dev/") { tty.to_string() } else { format!("/dev/{tty}") };
        Ok(Some((tty, stat.contains('+'))))
    }

    /// The pane a session's process runs in, checked: same terminal, still
    /// in its foreground. None when it can't be sure.
    pub fn locate(&self, c: &mut dyn Calls, pid: &Arg) -> Result<Option<(Pane, f64)>> {
        let Arg::Num(pid) = *pid else { return Ok(None) };
        if self.off || pid.fract() != 0.0 || !pid.is_finite() || pid <= 1.0 {
            return Ok(None);
        }
        let Some((tty, foreground)) = Self::proc(c, pid)? else { return Ok(None) };
        if !foreground {
            return Ok(None);
        }
        Ok(self.panes(c)?.into_iter().find(|p| p.tty.as_str() == Some(tty.as_str())).map(|p| (p, pid)))
    }

    pub fn deliver(&self, c: &mut dyn Calls, pid: &Arg, text: &Arg) -> Result<Arg> {
        let no = |why: &str| Arg::Obj(vec![("ok".into(), Arg::Bool(false)), ("why".into(), Arg::text(why))]);
        let Some((first, _)) = self.locate(c, pid)? else {
            return Ok(no("the session isn't in the foreground of a tmux pane XenoNet can see"));
        };
        let line = sanitize(&if text.is_nullish() { String::new() } else { js_text(text) }, true, MAX_TYPED);
        // Check again right before typing: the pane could have changed hands.
        let again = self.locate(c, pid)?;
        if !again.is_some_and(|(a, _)| a.pane == first.pane && a.tty == first.tty) {
            return Ok(no("the pane changed before XenoNet could type"));
        }
        let pane = js_text(&first.pane);
        let (typed, _) = exec(c, "tmux", &["send-keys", "-t", &pane, "-l", "--", &line])?;
        if !typed {
            return Ok(no("tmux didn't take the text"));
        }
        exec(c, "tmux", &["send-keys", "-t", &pane, "Enter"])?;
        Ok(Arg::Obj(vec![("ok".into(), Arg::Bool(true)), ("where".into(), first.pane)]))
    }
}
