//! Typing a message into the tmux pane a session runs in: the last resort
//! for a harness with no way in, and only with your consent for that
//! session (policy): core/delivery/tmux.js. Before every keystroke XenoNet
//! checks again that the pane's terminal still runs the session's own
//! process in the foreground, so it never types into the wrong place; the
//! text is one line, with no control characters or escape sequences, sent
//! literally (send-keys -l), then Enter.

use super::policy::sanitize;
use super::{number_text, Delivered, Error, Result};

pub const MAX_TYPED: f64 = 2000.0;

/// The commands tmux delivery runs: tmux and ps.
pub trait Shell {
    /// A command's output, or None when it failed to run or failed.
    fn exec(&mut self, cmd: &str, args: &[&str]) -> Option<String>;
}

/// The JS's split(/\s+/).
fn words(s: &str) -> Vec<&str> {
    crate::re!(r"\s+").split(s).collect()
}

/// A pane: its id, process, terminal and command.
#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    pub pane: String,
    pub pid: f64,
    pub tty: Option<String>,
    pub command: Option<String>,
}

/// tmux list-panes' lines, as the JS split them.
pub fn parse_panes(out: &str) -> Vec<Pane> {
    xenonet_base::text::js_trim(out)
        .split('\n')
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut parts = l.split('\t');
            let pane = parts.next().unwrap_or_default().to_string();
            let pid = parts.next().map_or(f64::NAN, |p| xenonet_base::text::js_number(Some(&serde_json::Value::String(p.to_string()))));
            let tty = parts.next().map(str::to_string);
            let command = parts.next().map(str::to_string);
            Pane { pane, pid, tty, command }
        })
        .collect()
}

pub struct Tmux {
    /// No typing here (Windows).
    pub off: bool,
}

impl Tmux {
    pub fn panes(&self, sh: &mut dyn Shell) -> Vec<Pane> {
        match sh.exec("tmux", &["list-panes", "-a", "-F", "#{pane_id}\t#{pane_pid}\t#{pane_tty}\t#{pane_current_command}"]) {
            Some(out) => parse_panes(&out),
            None => Vec::new(),
        }
    }

    /// A process's terminal and whether it's in the terminal's foreground
    /// process group (a + in its state).
    fn proc(sh: &mut dyn Shell, pid: f64) -> Result<Option<(String, bool)>> {
        let Some(out) = sh.exec("ps", &["-o", "tty=,stat=", "-p", &number_text(pid)]) else { return Ok(None) };
        let out = xenonet_base::text::js_trim(&out).to_string();
        if out.is_empty() {
            return Ok(None);
        }
        let w = words(&out);
        let tty = w[0];
        let Some(stat) = w.get(1) else {
            return Err(Error("Cannot read properties of undefined (reading 'includes')".into()));
        };
        let tty = if tty.starts_with("/dev/") { tty.to_string() } else { format!("/dev/{tty}") };
        Ok(Some((tty, stat.contains('+'))))
    }

    /// The pane a session's process runs in, checked: same terminal, still
    /// in its foreground. None when it can't be sure.
    pub fn locate(&self, sh: &mut dyn Shell, pid: f64) -> Result<Option<Pane>> {
        if self.off || pid.fract() != 0.0 || !pid.is_finite() || pid <= 1.0 {
            return Ok(None);
        }
        let Some((tty, foreground)) = Self::proc(sh, pid)? else { return Ok(None) };
        if !foreground {
            return Ok(None);
        }
        Ok(self.panes(sh).into_iter().find(|p| p.tty.as_deref() == Some(tty.as_str())))
    }

    /// The message typed into the session's pane: the pane's id, or why
    /// not.
    pub fn deliver(&self, sh: &mut dyn Shell, pid: f64, text: &str) -> Result<Delivered> {
        let Some(first) = self.locate(sh, pid)? else {
            return Ok(Err("the session isn't in the foreground of a tmux pane XenoNet can see".into()));
        };
        let line = sanitize(text, true, MAX_TYPED);
        // Check again right before typing: the pane could have changed hands.
        let again = self.locate(sh, pid)?;
        if !again.is_some_and(|a| a.pane == first.pane && a.tty == first.tty) {
            return Ok(Err("the pane changed before XenoNet could type".into()));
        }
        if sh.exec("tmux", &["send-keys", "-t", &first.pane, "-l", "--", &line]).is_none() {
            return Ok(Err("tmux didn't take the text".into()));
        }
        sh.exec("tmux", &["send-keys", "-t", &first.pane, "Enter"]);
        Ok(Ok(first.pane))
    }
}
