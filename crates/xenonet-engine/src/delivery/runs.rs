//! Headless runs: one turn of Claude Code or Codex that XenoNet starts
//! itself, for a session it resumes (`resume`) and, later, for one it
//! starts: core/delivery/runs.js. How a program is started on each
//! platform, whether a run may start now, and which records and logs are
//! kept are here; starting the process, writing its log and watching it
//! are the runner's (the binary's, or the twin's).
//!
//! At most `CAP` runs go at once on a machine, one per session, and a turn
//! runs `TURN_MS` at most. Each run's output goes to `runs/<id>.log` in
//! XenoNet's state folder (`LOG_MAX` at most, kept a week), and its
//! record (when it started, its exit, how long it took) to
//! delivery-runs.json beside it, so a restart keeps counting the runs
//! still under way.

use crate::sql::Arg;
use crate::store::js_text;
use serde_json::{json, Value};

/// Runs at once on one machine.
pub const CAP: usize = 4;
/// One turn, at most.
pub const TURN_MS: f64 = 30.0 * 60e3;
/// A run's log, at most, in bytes.
pub const LOG_MAX: usize = 1 << 20;
/// How long logs and records are kept.
pub const KEEP_MS: f64 = 7.0 * 24.0 * 3600e3;
/// Records kept, at most.
const KEEP_RUNS: usize = 200;

/// How to start a program: what to run and its arguments, each passed as
/// it is (the platform quotes it), or with `verbatim`, one command line
/// for cmd.exe, passed as it's written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub verbatim: bool,
}

impl Launch {
    pub fn to_arg(&self) -> Arg {
        Arg::Obj(vec![
            ("program".into(), Arg::text(self.program.clone())),
            ("args".into(), Arg::List(self.args.iter().map(|a| Arg::text(a.clone())).collect())),
            ("verbatim".into(), Arg::Bool(self.verbatim)),
        ])
    }
}

/// cmd.exe by its full path, never by its name, which Windows looks for in
/// the folder a run starts in (the session's project) first: %ComSpec%,
/// else %SystemRoot%\System32\cmd.exe.
pub fn cmd_exe(comspec: Option<&str>, system_root: Option<&str>) -> String {
    match (comspec.filter(|c| !c.is_empty()), system_root.filter(|r| !r.is_empty())) {
        (Some(c), _) => c.to_string(),
        (None, Some(root)) => format!("{root}\\System32\\cmd.exe"),
        (None, None) => "C:\\Windows\\System32\\cmd.exe".into(),
    }
}

/// How to start `program` with `args` here, or why it can't be done
/// safely, with cmd.exe as this process's environment names it.
pub fn launch(program: &str, args: &[String], windows: bool) -> Result<Launch, String> {
    let var = |k: &str| std::env::var(k).ok();
    launch_with(program, args, windows, &cmd_exe(var("ComSpec").as_deref(), var("SystemRoot").as_deref()))
}

/// How to start `program` with `args` here, or why it can't be done
/// safely. Elsewhere each argument goes as it is, and the platform quotes
/// it once. On Windows a batch file (an npm install's claude.cmd) runs only
/// through `cmd` (cmd.exe's full path), which takes one command line as
/// written: each part is quoted once, and a part cmd.exe would still read
/// inside quotes is refused: a quote, a % or a ! (which it expands), a line
/// break, or a \ at its end (which would escape the closing quote).
pub fn launch_with(program: &str, args: &[String], windows: bool, cmd: &str) -> Result<Launch, String> {
    let batch = program.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"));
    if !windows || !batch {
        return Ok(Launch { program: program.to_string(), args: args.to_vec(), verbatim: false });
    }
    let parts: Vec<&str> = std::iter::once(program).chain(args.iter().map(String::as_str)).collect();
    if let Some(bad) = parts.iter().find(|p| p.contains(['"', '%', '!', '\r', '\n']) || p.ends_with('\\')) {
        return Err(format!("XenoNet can't pass {bad} to a batch file safely"));
    }
    let line = parts.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(" ");
    Ok(Launch { program: cmd.into(), args: vec!["/d".into(), "/s".into(), "/c".into(), format!("\"{line}\"")], verbatim: true })
}

/// One run, as delivery-runs.json keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub id: String,
    /// What it's for: the session it resumes.
    pub key: String,
    pub app: String,
    pub pid: Option<f64>,
    /// When it started and ended, and how long it took, in ms.
    pub at: f64,
    pub end: Option<f64>,
    /// Its exit code: none for one killed by a signal, or still going.
    pub exit: Option<f64>,
    pub ms: Option<f64>,
    /// Whether the turn's cap stopped it (`TURN_MS`).
    pub timed_out: bool,
}

fn opt(v: Option<f64>) -> Value {
    v.map_or(Value::Null, xenonet_base::json::number)
}

impl Run {
    /// As the file keeps it: `timedOut` only on a run the cap stopped.
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "id": self.id, "key": self.key, "app": self.app, "pid": opt(self.pid), "at": xenonet_base::json::number(self.at), "end": opt(self.end), "exit": opt(self.exit), "ms": opt(self.ms) });
        if self.timed_out {
            v["timedOut"] = Value::Bool(true);
        }
        v
    }

    /// A record as the file or the JS has it; None for one without its id,
    /// key or start.
    pub fn of(v: &Value) -> Option<Run> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let n = |k: &str| v.get(k).and_then(Value::as_f64);
        Some(Run {
            id: s("id")?,
            key: s("key")?,
            app: s("app").unwrap_or_default(),
            pid: n("pid"),
            at: n("at")?,
            end: n("end"),
            exit: n("exit"),
            ms: n("ms"),
            timed_out: v.get("timedOut").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// Whether it counts as under way: not ended, within its turn, and its
    /// process alive (one a XenoNet before a restart started is watched by
    /// no one, but still runs).
    pub fn live(&self, now: f64, alive: &mut dyn FnMut(f64) -> bool) -> bool {
        self.end.is_none() && now - self.at < TURN_MS && self.pid.is_some_and(&mut *alive)
    }
}

/// The records in delivery-runs.json's text: none for a file that isn't
/// one.
pub fn read(text: &str) -> Vec<Run> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    v.get("runs").and_then(Value::as_array).map(|a| a.iter().filter_map(Run::of).collect()).unwrap_or_default()
}

/// delivery-runs.json's text for these records.
pub fn write(runs: &[Run]) -> String {
    json!({ "runs": runs.iter().map(Run::to_json).collect::<Vec<_>>() }).to_string()
}

/// Whether a run for `key` may start now: None, or why not.
pub fn admit(runs: &[Run], key: &str, now: f64, cap: usize, alive: &mut dyn FnMut(f64) -> bool) -> Option<String> {
    admit_with(runs, &[], key, now, cap, alive)
}

/// `admit`, counting `also` too: the keys of runs a runner has under way,
/// whether or not they have a record yet (one just admitted, still
/// starting), each once.
pub fn admit_with(runs: &[Run], also: &[String], key: &str, now: f64, cap: usize, alive: &mut dyn FnMut(f64) -> bool) -> Option<String> {
    let live: Vec<&Run> = runs.iter().filter(|r| !also.contains(&r.key) && r.live(now, alive)).collect();
    if also.iter().any(|k| k == key) || live.iter().any(|r| r.key == key) {
        return Some("a run is under way for it".into());
    }
    if live.len() + also.len() >= cap {
        return Some(format!("XenoNet runs at most {cap} turns at once on this machine"));
    }
    None
}

/// The records worth keeping: the last week's, at most 200, the newest
/// last.
pub fn kept(runs: &[Run], now: f64) -> Vec<Run> {
    let recent: Vec<Run> = runs.iter().filter(|r| now - r.at < KEEP_MS).cloned().collect();
    let from = recent.len().saturating_sub(KEEP_RUNS);
    recent[from..].to_vec()
}

/// The logs to delete, of the files in the runs folder (name, mtime): those
/// last written a week ago or more.
pub fn stale_logs(files: &[(String, f64)], now: f64) -> Vec<String> {
    files.iter().filter(|(name, mtime)| name.rsplit_once('.').is_some_and(|(_, ext)| ext == "log") && now - mtime >= KEEP_MS).map(|(name, _)| name.clone()).collect()
}

/// A record as the JS passed it.
pub fn run_of(a: &Arg) -> Option<Run> {
    Run::of(&a.to_json())
}

/// What a file's name is, as the JS passed it: { name, mtime }.
pub fn file_of(a: &Arg) -> (String, f64) {
    (js_text(&a.field("name")), a.field("mtime").as_f64().unwrap_or(f64::NAN))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn a_program_is_quoted_once_for_its_platform() {
        let args = s(&["-p", "--resume", "0190d3e0-0000-7000-8000-00000000ab01", "--settings", "/Users/Jo Smith/listen.json"]);
        // macOS and Linux: as it is; the platform passes each argument whole.
        let mac = launch("/Users/Jo Smith/.local/bin/claude", &args, false).unwrap();
        assert_eq!((mac.program.as_str(), &mac.args, mac.verbatim), ("/Users/Jo Smith/.local/bin/claude", &args, false));
        // Windows, an .exe: as it is too; Rust or Node quotes it once.
        let exe = launch("C:\\Program Files\\Claude\\claude.exe", &args, true).unwrap();
        assert_eq!((exe.program.as_str(), exe.verbatim), ("C:\\Program Files\\Claude\\claude.exe", false));
        // Windows, a .cmd: through cmd.exe by its full path, as one line,
        // each part quoted.
        let shell = "C:\\Windows\\System32\\cmd.exe";
        let cmd = launch_with("C:\\Users\\Jo Smith\\AppData\\Roaming\\npm\\claude.CMD", &s(&["-p", "--settings", "C:\\Users\\Jo Smith\\listen.json"]), true, shell).unwrap();
        assert_eq!(
            cmd,
            Launch {
                program: shell.into(),
                args: s(&["/d", "/s", "/c", "\"\"C:\\Users\\Jo Smith\\AppData\\Roaming\\npm\\claude.CMD\" \"-p\" \"--settings\" \"C:\\Users\\Jo Smith\\listen.json\"\""]),
                verbatim: true
            }
        );
        assert_eq!(launch_with("C:\\npm\\codex.bat", &s(&["exec", "-"]), true, shell).unwrap().args[3], "\"\"C:\\npm\\codex.bat\" \"exec\" \"-\"\"");
        assert_eq!(launch_with("C:\\npm\\claude.cmd", &s(&["100%"]), true, shell), Err("XenoNet can't pass 100% to a batch file safely".into()));
        for bad in ["a\"b", "hi!", "C:\\dir\\", "a\nb"] {
            assert!(launch_with("C:\\npm\\claude.cmd", &s(&[bad]), true, shell).is_err(), "{bad}");
        }
        assert!(!launch("/opt/claude.cmd", &args, false).unwrap().verbatim, "only Windows runs batch files");
        // cmd.exe is %ComSpec%, else in %SystemRoot%, never found by name.
        assert_eq!(cmd_exe(Some("D:\\W\\system32\\cmd.exe"), Some("C:\\Windows")), "D:\\W\\system32\\cmd.exe");
        assert_eq!(cmd_exe(Some(""), Some("E:\\Win")), "E:\\Win\\System32\\cmd.exe");
        assert_eq!(cmd_exe(None, None), "C:\\Windows\\System32\\cmd.exe");
    }

    fn run(id: &str, key: &str, at: f64, end: Option<f64>) -> Run {
        Run { id: id.into(), key: key.into(), app: "claude".into(), pid: Some(10.0), at, end, exit: None, ms: None, timed_out: false }
    }

    #[test]
    fn four_runs_at_once_one_per_session_and_a_restart_keeps_counting() {
        let now = 1_000_000.0;
        let mut alive = |_: f64| true;
        let mut runs = vec![run("r1", "claude:a", now - 1000.0, None), run("r2", "claude:b", now - 1000.0, Some(now - 10.0))];
        assert_eq!(admit(&runs, "claude:a", now, CAP, &mut alive).as_deref(), Some("a run is under way for it"));
        assert_eq!(admit(&runs, "claude:b", now, CAP, &mut alive), None, "its run ended");
        for k in ["c", "d", "e"] {
            runs.push(run(k, &format!("claude:{k}"), now - 5.0, None));
        }
        assert_eq!(admit(&runs, "claude:f", now, CAP, &mut alive).as_deref(), Some("XenoNet runs at most 4 turns at once on this machine"));
        assert_eq!(admit(&runs, "claude:f", now + TURN_MS, CAP, &mut alive), None, "past their turn they don't count");
        assert_eq!(admit(&runs, "claude:f", now, CAP, &mut |pid| pid != 10.0), None, "nor once their processes are gone");
        // A runner's own runs under way count once, record or not.
        let mine = s(&["claude:a", "claude:g"]);
        assert_eq!(admit_with(&runs[..2], &mine, "claude:g", now, CAP, &mut alive).as_deref(), Some("a run is under way for it"), "admitted, still starting");
        assert_eq!(admit_with(&runs[..2], &mine, "claude:h", now, CAP, &mut alive), None, "claude:a counted once");
        assert!(admit_with(&runs[..2], &s(&["claude:a", "claude:g", "claude:h"]), "claude:i", now, 3, &mut alive).is_some());
        // What the file keeps comes back the same, a timed-out run too.
        runs[1].timed_out = true;
        assert_eq!(runs[1].to_json()["timedOut"], true);
        assert!(runs[0].to_json().get("timedOut").is_none());
        let back = read(&write(&runs));
        assert_eq!(back, runs);
        assert_eq!(read("not json"), []);
        let old = run("old", "claude:z", now - KEEP_MS, Some(now - KEEP_MS));
        assert_eq!(kept(&[old, runs[0].clone()], now), [runs[0].clone()]);
        let many: Vec<Run> = (0..250).map(|i| run(&format!("r{i}"), "claude:a", now - 1.0, Some(now))).collect();
        assert_eq!(kept(&many, now).first().map(|r| r.id.as_str()), Some("r50"));
        let files = [("run_a.log".to_string(), now - KEEP_MS), ("run_b.log".into(), now - 1.0), ("notes.txt".into(), 0.0)];
        assert_eq!(stale_logs(&files, now), ["run_a.log"]);
    }
}
