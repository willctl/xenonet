//! The doctor's view of Adytum and Eidolon, the two tools that keep cells
//! and identities beside XenoNet, each in its own repository. Adytum's own
//! checks run in this process, through `adytum-doctor` as a library, with a
//! short deadline. Eidolon is driven through its command, never its crates:
//! the doctor looks for `eidolon` on the PATH it uses for tools, asks it for
//! its own `--json` answers with an argument list and a short deadline, and
//! turns what comes back into a verdict. Eidolon not being there is an info
//! with the line that builds it, never a failure of XenoNet's doctor.
//! Nothing secret is read or shown: Eidolon's holder entry holds only
//! public keys and only its name is used here.

use crate::checks::{info, ok, unreadable, warn, Ctx};
use serde_json::{json, Value};
use xenonet_os::error::Failure;
use xenonet_text::units::plural;

/// The checks, in order: id, group and title.
pub const CHECKS: [(&str, &str, &str); 2] =
    [("cells.adytum", "Integrations", "Adytum can keep cells"), ("cells.eidolon", "Integrations", "Eidolon holds this machine's identities")];

/// How to get Eidolon's program, from a checkout of its repository.
const BUILD: &str = "From a checkout of github.com/willctl/eidolon run cargo install --path crates/eidolon-cli.";

const ADYTUM_MS: f64 = 6000.0;
const EIDOLON_MS: f64 = 3000.0;
/// What one finding or one eidolon may say, and how many are listed.
const SAID_MAX: usize = 160;
const LISTED: usize = 5;

/// The demo's made-up answers, in the tools' own shapes: one warning from
/// Adytum, a machine called `desk`, one eidolon open and one sealed.
const DEMO_ADYTUM: &str = r#"{"ok":true,"checks":[
  {"id":"limactl","ok":true,"level":"info","message":"limactl 2.1.2 is installed.","fix":null},
  {"id":"disk","ok":false,"level":"warn","message":"45 GB are free; a VM wants 20 GB to start with room to spare.","fix":"Free some space before making another cell."}]}"#;
const DEMO_MACHINE: &str = r#"{"name":"desk","sign":"demo","seal":"demo"}"#;
const DEMO_LS: &str = r#"{"ok":true,"eidolons":[
  {"eidolon":"eid_demo0000000000000001","wall":"wall_demo","state":{"open":{"cell":"adytum-demo-1"}},"generation":1,"seat_here":true,"pending_move":false,"facets":2},
  {"eidolon":"eid_demo0000000000000002","wall":"wall_demo","state":"sealed","generation":3,"seat_here":true,"pending_move":false,"facets":1}]}"#;

pub(crate) async fn run(c: &Ctx<'_>, id: &str) -> Result<Value, Failure> {
    let demo = c.f.data.get("demo").and_then(Value::as_bool) == Some(true);
    match id {
        "cells.adytum" => Ok(adytum(demo).await),
        "cells.eidolon" => Ok(eidolon(c, demo).await),
        other => Err(Failure::msg(format!("no such check: {other}"))),
    }
}

/// One program's answer: its output, or why there is none.
enum Said {
    Out {
        stdout: String,
        stderr: String,
    },
    /// It started and didn't finish in time, or couldn't start.
    Late,
    Gone,
}

async fn ask(c: &Ctx<'_>, file: &str, args: &[&str], ms: f64) -> Said {
    let r = c.run(file, args, ms).await;
    if r.get("timedOut").and_then(Value::as_bool) == Some(true) {
        return Said::Late;
    }
    let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    // A program that can't start answers with a code that is a word.
    if r.get("code").is_some_and(Value::is_string) {
        return Said::Gone;
    }
    Said::Out { stdout: text("stdout"), stderr: text("stderr") }
}

async fn adytum(demo: bool) -> Value {
    if demo {
        return adytum_verdict(DEMO_ADYTUM);
    }
    match tokio::time::timeout(std::time::Duration::from_millis(ADYTUM_MS as u64), adytum_doctor::checks()).await {
        Ok(checks) => adytum_checks_verdict(&checks),
        Err(_) => warn(format!("adytum didn't answer within {} seconds", ADYTUM_MS / 1000.0), Some("Run adytum doctor yourself to see where it waits.")),
    }
}

async fn eidolon(c: &Ctx<'_>, demo: bool) -> Value {
    if demo {
        return eidolon_verdict(DEMO_MACHINE, "", DEMO_LS);
    }
    let Some(file) = c.which("eidolon").await else { return absent("Eidolon") };
    let (machine, ls) = (ask(c, &file, &["machine", "show", "--json"], EIDOLON_MS).await, ask(c, &file, &["ls", "--json"], EIDOLON_MS).await);
    match (machine, ls) {
        (Said::Out { stdout: m, stderr: e }, Said::Out { stdout: l, .. }) => eidolon_verdict(&m, &e, &l),
        (Said::Late, _) | (_, Said::Late) => warn(format!("eidolon didn't answer within {} seconds", EIDOLON_MS / 1000.0), Some("Run eidolon ls yourself to see where it waits.")),
        _ => warn("eidolon is on the PATH but doesn't start", Some(BUILD)),
    }
}

fn absent(tool: &str) -> Value {
    info(format!("{tool} isn't installed here, so XenoNet shows nothing of it"), Some(BUILD))
}

// Text from a tool, safe to show on one line: control characters become
// spaces and it is cut short.
fn tidy(s: &str) -> String {
    let flat: String = s.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > SAID_MAX {
        let cut: String = flat.chars().take(SAID_MAX).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

fn parse(s: &str) -> Option<Value> {
    serde_json::from_str(s.trim()).ok()
}

fn more(n: usize) -> String {
    if n > LISTED {
        format!(" and {} more", n - LISTED)
    } else {
        String::new()
    }
}

/// Adytum's checks as a verdict, through the same JSON `adytum doctor
/// --json` prints.
pub fn adytum_checks_verdict(checks: &[adytum_doctor::Check]) -> Value {
    adytum_verdict(&adytum_doctor::render_json(checks))
}

/// `adytum doctor --json`'s answer as a verdict: what is failing or
/// worth a warning, each with the tool's own fix.
pub fn adytum_verdict(stdout: &str) -> Value {
    let Some(checks) = parse(stdout).and_then(|v| v.get("checks").cloned()).and_then(|c| c.as_array().cloned()) else { return warn(unreadable("adytum doctor"), None) };
    let level = |x: &Value| x.get("level").and_then(Value::as_str).unwrap_or_default().to_string();
    let wrong: Vec<&Value> = checks.iter().filter(|x| x.get("ok").and_then(Value::as_bool) == Some(false) && matches!(level(x).as_str(), "warn" | "fail")).collect();
    if wrong.is_empty() {
        return ok(format!("Adytum: {} passed", plural(checks.len() as f64, "check", None)));
    }
    let fails = wrong.iter().filter(|x| level(x) == "fail").count();
    let warns = wrong.len() - fails;
    let mut head = Vec::new();
    if fails > 0 {
        head.push(plural(fails as f64, "check failing", Some("checks failing")));
    }
    if warns > 0 {
        head.push(plural(warns as f64, "warning", None));
    }
    // Failures first.
    let mut sorted = wrong.clone();
    sorted.sort_by_key(|x| level(x) != "fail");
    let said =
        |x: &&Value| format!("{} ({}): {}", tidy(x.get("id").and_then(Value::as_str).unwrap_or("?")), level(x), tidy(x.get("message").and_then(Value::as_str).unwrap_or("")));
    let detail = format!("Adytum: {}. {}{}", head.join(", "), sorted.iter().take(LISTED).map(said).collect::<Vec<_>>().join("; "), more(sorted.len()));
    let fixes: Vec<String> = sorted.iter().filter_map(|x| x.get("fix").and_then(Value::as_str)).map(tidy).filter(|f| !f.is_empty()).take(LISTED).collect();
    // Adytum being unwell doesn't stop XenoNet, so it is a warning here.
    warn(detail, (!fixes.is_empty()).then(|| fixes.join(" ")).as_deref())
}

/// Eidolon's machine entry and holdings as a verdict. `machine` and `err`
/// are `eidolon machine show --json`'s stdout and stderr, `ls` is
/// `eidolon ls --json`'s stdout.
pub fn eidolon_verdict(machine: &str, err: &str, ls: &str) -> Value {
    let name = match parse(machine).and_then(|m| m.get("name").and_then(Value::as_str).map(tidy)) {
        Some(n) => Some(n),
        // With --json the refusal comes on stdout; stderr is read too.
        None => match parse(machine).or_else(|| parse(err)) {
            // Not set up yet is a fact, not a fault.
            Some(e) if e.get("code").and_then(Value::as_str) == Some("not_initialised") => None,
            Some(e) if e.get("ok") == Some(&json!(false)) => {
                let said = |k: &str| tidy(e.get(k).and_then(Value::as_str).unwrap_or_default());
                return warn(format!("Eidolon can't read this machine's identity: {}", said("message")), Some(&said("fix")).filter(|f| !f.is_empty()).map(String::as_str));
            }
            _ => return warn(unreadable("eidolon machine show"), None),
        },
    };
    let Some(rows) = parse(ls).and_then(|v| v.get("eidolons").and_then(Value::as_array).cloned()) else { return warn(unreadable("eidolon ls"), None) };
    let who = name.as_ref().map_or_else(|| "This machine has no identity yet".to_string(), |n| format!("This machine is {n}"));
    let mut open = Vec::new();
    let (mut sealed, mut retired) = (0, 0);
    for r in &rows {
        let id = tidy(r.get("eidolon").and_then(Value::as_str).unwrap_or("?"));
        match r.get("state") {
            Some(Value::String(s)) if s == "sealed" => sealed += 1,
            Some(Value::String(s)) if s == "retired" => retired += 1,
            Some(s) => match s.get("open").and_then(|o| o.get("cell")).and_then(Value::as_str) {
                Some(cell) => open.push(format!("{id} in {}", tidy(cell))),
                None => sealed += 1,
            },
            None => sealed += 1,
        }
    }
    let held = plural(rows.len() as f64, "eidolon", None);
    let mut detail = format!("{who}; it holds {held}");
    if !rows.is_empty() {
        let retired = if retired > 0 { format!(", {retired} retired") } else { String::new() };
        detail = format!("{detail} ({} open, {sealed} sealed{retired})", open.len());
    }
    if !open.is_empty() {
        detail = format!("{detail}. Open: {}{}", open.iter().take(LISTED).cloned().collect::<Vec<_>>().join("; "), more(open.len()));
    }
    if name.is_none() {
        return info(detail, Some("Run eidolon machine init --name <name> to give this machine an identity."));
    }
    ok(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{self, Facts};
    use crate::fake::{ready, Fake};

    const HOLDER: &str = r#"{"name":"desk","sign":"AAAA","seal":"BBBB"}"#;
    const NOT_SET_UP: &str = r#"{"ok":false,"code":"not_initialised","message":"This machine has no identity yet.","fix":"Run `eidolon machine init --name <name>` first."}"#;

    #[test]
    fn adytum_lists_its_fails_and_warns_with_their_fixes() {
        let v = adytum_verdict(
            r#"{"ok":false,"checks":[
            {"id":"limactl","ok":false,"level":"fail","message":"limactl isn't installed.","fix":"Install Lima."},
            {"id":"disk","ok":false,"level":"warn","message":"Low disk.","fix":"Free space."},
            {"id":"note","ok":false,"level":"info","message":"Only a note.","fix":null},
            {"id":"fine","ok":true,"level":"info","message":"fine","fix":null}]}"#,
        );
        assert_eq!(v["status"], "warn");
        assert_eq!(v["detail"], "Adytum: 1 check failing, 1 warning. limactl (fail): limactl isn't installed.; disk (warn): Low disk.");
        assert_eq!(v["fix"], "Install Lima. Free space.");
        let all_good = adytum_verdict(r#"{"ok":true,"checks":[{"id":"a","ok":true,"level":"info","message":"m","fix":null}]}"#);
        assert_eq!((all_good["status"].as_str(), all_good["detail"].as_str()), (Some("ok"), Some("Adytum: 1 check passed")));
    }

    #[test]
    fn adytum_answers_that_cannot_be_read_are_a_warning_not_a_crash() {
        for bad in ["", "not json", "{}", r#"{"checks":"x"}"#, "[1]"] {
            let v = adytum_verdict(bad);
            assert_eq!(v["status"], "warn", "{bad}");
            assert!(v["detail"].as_str().unwrap().contains("adytum doctor answered in a way XenoNet doesn't read"));
        }
    }

    #[test]
    fn eidolon_says_who_it_is_and_what_is_open_where() {
        let ls = r#"{"ok":true,"eidolons":[
          {"eidolon":"eid_a","state":{"open":{"cell":"adytum-one"}}},
          {"eidolon":"eid_b","state":"sealed"},
          {"eidolon":"eid_c","state":"retired"}]}"#;
        let v = eidolon_verdict(HOLDER, "", ls);
        assert_eq!(v["status"], "ok");
        assert_eq!(v["detail"], "This machine is desk; it holds 3 eidolons (1 open, 1 sealed, 1 retired). Open: eid_a in adytum-one");
        assert!(!v.to_string().contains("AAAA") && !v.to_string().contains("BBBB"), "no key is shown");
        assert_eq!(eidolon_verdict(HOLDER, "", r#"{"ok":true,"eidolons":[]}"#)["detail"], "This machine is desk; it holds 0 eidolons");
    }

    #[test]
    fn eidolon_without_an_identity_is_an_info_with_the_command_that_makes_one() {
        let v = eidolon_verdict("", NOT_SET_UP, r#"{"ok":true,"eidolons":[]}"#);
        assert_eq!(eidolon_verdict(NOT_SET_UP, "", r#"{"ok":true,"eidolons":[]}"#)["status"], "info", "stdout carries it with --json");
        assert_eq!(v["status"], "info");
        assert!(v["fix"].as_str().unwrap().contains("eidolon machine init --name <name>"));
    }

    #[test]
    fn eidolon_answers_that_cannot_be_read_are_a_warning() {
        assert_eq!(eidolon_verdict("garbage", "", "{}")["status"], "warn");
        assert_eq!(eidolon_verdict(HOLDER, "", "garbage")["status"], "warn");
        let damaged = r#"{"ok":false,"code":"key_store","message":"This machine's keys are missing or damaged.","fix":"Restore the keys folder."}"#;
        let v = eidolon_verdict("", damaged, "{}");
        assert_eq!((v["status"].as_str(), v["fix"].as_str()), (Some("warn"), Some("Restore the keys folder.")));
    }

    #[test]
    fn what_a_tool_says_is_flattened_and_cut_before_it_is_shown() {
        assert_eq!(tidy("a\u{1b}[31m\nb\tc"), "a [31m b c");
        assert_eq!(tidy(&"x".repeat(400)).chars().count(), SAID_MAX + 1);
    }

    // The doctor's output with the tools there or not, through the real
    // check runner and a made-up machine.
    fn drive(id: &str, which: bool, run: impl Fn(&str, &[&str]) -> Value + 'static, demo: bool) -> Value {
        let host = Fake { run: Box::new(run), missing: if which { vec![] } else { vec!["eidolon"] }, ..Fake::default() };
        ready(checks::run(&host, id.into(), Facts { data: json!({ "platform": "darwin", "home": "/Users/ada", "demo": demo }) })).unwrap_or_else(|f| panic!("{}", f.message()))
    }

    #[test]
    fn a_missing_eidolon_is_an_info_with_the_build_line() {
        let v = drive("cells.eidolon", false, |_, _| unreachable!("nothing runs when the tool is missing"), false);
        assert_eq!(v["status"], "info");
        assert!(v["fix"].as_str().unwrap().contains("cargo install --path crates/eidolon-cli"));
    }

    #[test]
    fn adytums_own_checks_make_the_same_verdict_its_json_does() {
        use adytum_doctor::{Check, Level};
        let checks = [
            Check { id: "limactl".into(), ok: false, level: Level::Fail, message: "limactl isn't installed.".into(), fix: Some("Install Lima.".into()) },
            Check { id: "disk".into(), ok: false, level: Level::Warn, message: "Low disk.".into(), fix: Some("Free space.".into()) },
            Check { id: "fine".into(), ok: true, level: Level::Info, message: "fine".into(), fix: None },
        ];
        let v = adytum_checks_verdict(&checks);
        assert_eq!(v["status"], "warn");
        assert_eq!(v["detail"], "Adytum: 1 check failing, 1 warning. limactl (fail): limactl isn't installed.; disk (warn): Low disk.");
        assert_eq!(v["fix"], "Install Lima. Free space.");
        assert_eq!(adytum_checks_verdict(&checks[2..])["detail"], "Adytum: 1 check passed");
    }

    #[test]
    fn eidolon_is_asked_with_argument_lists() {
        let asked = |file: &str, args: &[&str]| -> Value {
            match args {
                ["machine", "show", "--json"] => json!({ "code": 0, "stdout": HOLDER, "stderr": "" }),
                ["ls", "--json"] => json!({ "code": 0, "stdout": r#"{"ok":true,"eidolons":[{"eidolon":"eid_a","state":"sealed"}]}"#, "stderr": "" }),
                other => panic!("{file} {other:?}"),
            }
        };
        assert_eq!(drive("cells.eidolon", true, asked, false)["detail"], "This machine is desk; it holds 1 eidolon (0 open, 1 sealed)");
    }

    #[test]
    fn an_eidolon_that_hangs_or_cannot_start_is_a_warning() {
        let v = drive("cells.eidolon", true, |_, _| json!({ "code": null, "stdout": "", "stderr": "", "timedOut": true }), false);
        assert!(v["detail"].as_str().unwrap().contains("didn't answer within 3 seconds"), "{v}");
        let v = drive("cells.eidolon", true, |_, _| json!({ "code": "ENOENT", "stdout": "", "stderr": "", "timedOut": false }), false);
        assert_eq!(v["status"], "warn");
    }

    #[test]
    fn the_demo_shows_made_up_data_and_runs_nothing() {
        let a = drive("cells.adytum", true, |_, _| unreachable!("the demo runs no program"), true);
        assert_eq!(a["status"], "warn");
        let e = drive("cells.eidolon", false, |_, _| unreachable!("the demo runs no program"), true);
        assert_eq!(e["status"], "ok");
        assert!(e["detail"].as_str().unwrap().starts_with("This machine is desk; it holds 2 eidolons"), "{e}");
    }
}
